# GUI toolkit for the viewer

Research for ticket `.scratch/diff-viewer-mvp/issues/01-gui-toolkit-research.md`.
All version numbers, dates, and activity counts were checked on **2026-10-01**, using crates.io API, GitHub API, and official docs. They will go stale.

## Recommendation

**Pick: gpui** (Zed's UI framework). Depend on an **exactly pinned Zed git tag**, for example `gpui = { git = "https://github.com/zed-industries/zed", tag = "v1.22.0" }`, plus `gpui_platform` from the same tag. Do not use the crates.io `gpui` crate.

**Runner-up: egui** (with `eframe`).

**When the runner-up wins.** The deciding question is project policy: will the project accept a core dependency that has no maintained semver release on crates.io? gpui is only usable as a git pin or through a third-party republisher. If the answer is no, egui is the pick.

Why gpui wins on evidence:

- **Built for this workload.** It is the renderer of a production code editor, and Zed has shipped side-by-side split diffs by default since v0.224 (Feb 2026).
- **The primitives we need are already there:**
  - `TextRun` carries both `color` and `background_color`. That covers syntax colors and Inline Change backgrounds in one pass.
  - `uniform_list` virtualizes fixed-height rows.
  - `PathBuilder::cubic_bezier_to` plus `window.paint_path` inside a `canvas` element covers the Connectors.
- **Diff and git apps already ship on it outside Zed:** GitComet and JayJay.
- **An Apache-2.0 editor component exists.** gpui-component claims 200K-line Tree-sitter editing.

Why gpui loses on criteria (4) and (5):

- The official crates.io release is a year old (0.2.2, 2025-10-22).
- The README's own install instructions name `gpui_platform`, a crate that is **not on crates.io** (it returns 404).
- Three separate community redistributions exist.
- A Zed team member wrote in Mar 2026 that the team does not have the resources to maintain gpui as a standalone library.

egui is the safest standalone choice: monthly-ish releases, 24.9M downloads, and the bus factor isn't a third-party republisher. It covers criteria (1) and (2) only with extra work:

- Filled paths are **convex-only**, so Connector bands need a hand-built `Mesh`.
- Immediate mode means caching laid-out text (galleys) ourselves.

iced, Tauri, Slint, Floem, Xilem, and Makepad are each ruled out for a specific reason. See the table and per-toolkit sections.

**Next step:** a one-day spike on gpui and egui, run on Hyprland and macOS. Measure:

- opening 100k lines with per-span colors and backgrounds, against the 300ms target;
- two independently scrolling panes;
- one filled bezier Connector.

No toolkit below has a measured number for our workload.

## Comparison

Rating key: ✅ fits as-is · 🟡 workable with known effort or risk · ❌ blocker or major risk.

| | (1) Custom painting: Connectors | (2) Large text | (3) Wayland (Hyprland) / macOS | (4) Standalone crate | (5) Stability / maintenance |
|---|---|---|---|---|---|
| **gpui** | ✅ `PathBuilder` (lyon) with `cubic_bezier_to`, fill or stroke, inside `canvas` | ✅ `TextRun{color, background_color}`, `uniform_list`, editor-grade (Zed) | 🟡 Zed ships on Linux (Wayland/X11) and macOS (Metal); some Wayland scaling bugs | ❌ crates.io stale; must git-pin or use a third-party republish | 🟡 very active, but pre-1.0, "often breaking changes" |
| **egui** | 🟡 `CubicBezierShape` exists, but fill is convex-only, so build a `Mesh` | 🟡 `LayoutJob`/`TextFormat{color, background}`, `ScrollArea::show_rows`; per-frame cost, so cache galleys | 🟡 winit/eframe, widely used; macOS supported; open Hyprland reports (keyboard input ignored, NVIDIA exit segfault) | ✅ crates.io, docs.rs, 0.36.2 (2026-09-08) | 🟡 0.x with breaking minors about every 1–2 months; very active |
| **iced** | ✅ `canvas` `Path::bezier_curve_to` (lyon) | 🟡 `Span{color, highlight}` fits; **no upstream virtual list** (pop-os fork has one); text perf issues on record | ✅ COSMIC desktop (stable since Dec 2025) is built on it | ✅ crates.io | 🟡 last release 0.14.0 on 2025-12-07; 0.15 unreleased |
| **Tauri** | ✅ SVG/canvas is trivial | 🟡 CodeMirror-class virtualization exists in JS; webview perf is the issue | ❌ WebKitGTK on Linux: documented blank windows, Wayland protocol errors, NVIDIA/DMABUF issues | ✅ | ✅ 2.12.1 stable; CEF runtime only in 3.0 alpha |
| **Slint** | ✅ `Path` with `CubicTo`, fill rules | ❌ `StyledText` has **no per-span background**; markup-based input | ✅ | ✅ 1.18.1; GPL / royalty-free / commercial | ✅ 1.x semver, regular releases |
| **Floem** | 🟡 not verified in depth | 🟡 powers Lapce's editor | 🟡 | ❌ crates.io 0.2.0 is from 2024-11; Lapce itself git-pins | ❌ 5 commits from 2026-04 to 2026-10 |
| **Xilem/Masonry** | 🟡 Vello | 🟡 Parley | 🟡 | 🟡 0.4.0 (2025-10) | ❌ self-described "experimental" |
| **Makepad** | ✅ shader DSL | 🟡 has a code editor (Studio) | 🟡 | ❌ crates.io 1.0.0 (2025-05) vs. active `dev` branch | 🟡 active, but its own DSL/tooling, AI/game focus |

## Cross-cutting finding: no toolkit gives Connectors for free

Zed's split diff aligns the two sides with **spacers** ("empty visual regions, rendered with a subtle checkerboard") and scrolls them in sync. It does not draw IntelliJ-style curved Connectors between independently scrolling panes ([Zed blog, 2026-02-18](https://zed.dev/blog/split-diffs)). For every toolkit, Connectors are custom painting in a gutter element. That element reads the scroll offsets of both panes and draws one filled band per Change Block. Each Change Block's Connector is a non-convex shape: two cubic curves joined by two vertical edges. That's why the egui convex-fill limit below matters.

## Per-toolkit findings

### gpui

**What it is.** "A hybrid immediate and retained mode, GPU accelerated, UI framework for Rust". The README warns it "is still pre-1.0. There will often be breaking changes between versions" ([gpui README on main](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)). License: Apache-2.0 ([crates/gpui/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml)).

**(1) Custom painting.**
- `crates/gpui/src/path_builder.rs` exposes `PathBuilder::fill()` / `stroke(width)` with `move_to`, `line_to`, `curve_to`, `cubic_bezier_to`, `arc_to`, `close`, and `build()`. It is backed by lyon tessellation ([source](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/path_builder.rs)).
- The official `painting.rs` example draws curves with `builder.curve_to(...)` and paints them inside a `canvas(...)` element with `window.paint_path(path, color)` ([example](https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/painting.rs)).
- That is exactly the Connector gutter pattern.

**(2) Large text.**
- `TextRun` has `len`, `font`, `color`, `background_color: Option<Hsla>`, `underline`, and `strikethrough`. `StyledText::with_runs` / `with_highlights` take per-span styling ([elements/text.rs](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/elements/text.rs)). Syntax colors and Inline Change backgrounds fit in one text element.
- `uniform_list`: "provides lazy rendering for a set of items that are of uniform height … will only render the visible subset of items". It skips full taffy layout for speed ([elements/uniform_list.rs](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/elements/uniform_list.rs)). A variable-height `list` element also exists. Fixed-height code lines suit `uniform_list`.
- Existence proofs:
  - Zed's split diff ([blog](https://zed.dev/blog/split-diffs)).
  - **GitComet**, a gpui git GUI with diff/merge tooling, "opens the Chromium repository in about a second" ([gitcomet.dev](https://gitcomet.dev/), [repo](https://github.com/Auto-Explore/GitComet)).
  - **JayJay**, a Jujutsu GUI with "side-by-side diffs" ([repo](https://github.com/hewigovens/jayjay)).
  - **gpui-component** advertises "Code Editor: Stable performance at 200K lines with Tree-sitter highlighting", and "Virtual Lists … including lists whose items have different sizes". It is Apache-2.0 and used in the commercial Longbridge Pro app ([README](https://github.com/longbridge/gpui-component)).
  - These are vendor claims, not measurements.
- **License trap:** Zed's own `editor` crate is `GPL-3.0-or-later` ([crates/editor/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/editor/Cargo.toml)). We can depend on gpui (Apache-2.0) but must not copy Zed's editor or diff code unless the viewer is GPL.

**(3) Platforms.**
- macOS renders via Metal.
- Linux needs the `wayland` and/or `x11` features on `gpui_platform` ([README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)).
- `gpui_linux` pulls in `wayland-client`, `wayland-protocols`, `-wlr`, and `-plasma`, and renders through `gpui_wgpu` ([gpui_linux/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui_linux/Cargo.toml)). Zed is a daily driver on Linux.
- Known Wayland rough edges:
  - A Hyprland "no window appears" report, closed as *not planned* on 2026-03-18 ([#37918](https://github.com/zed-industries/zed/issues/37918)).
  - A fractional-scaling blur report opened today, on a Smithay compositor, not Hyprland ([#65032](https://github.com/zed-industries/zed/issues/65032)).

**(4) Standalone use: the weak point.** The current options:

| Way to consume gpui | Version / date | Publisher | Notes |
|---|---|---|---|
| crates.io `gpui` (official) | 0.2.2, 2025-10-22 | Zed | ~1 year stale. Current README says to also add `gpui_platform`, which **404s on crates.io** ([crates.io](https://crates.io/crates/gpui), [README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)) |
| Zed git tag/rev | e.g. `v1.22.0` (2026-09-30) | Zed | What JayJay does (`tag = "v1.20.2"`, [Cargo.toml](https://github.com/hewigovens/jayjay/blob/main/Cargo.toml)). Upstream code, but a large clone and no semver |
| `gpui-pre` | 0.3.7, 2026-09-28, weekly | huacnlee (Longbridge), single owner | "gpui-pre snapshot of zed@1a28cff"; gpui-component pins `=0.3.7` because a caret requirement broke apps (#3156) ([crates.io](https://crates.io/crates/gpui-pre), [gpui-component Cargo.toml](https://github.com/longbridge/gpui-component/blob/main/Cargo.toml)) |
| `gpui-unofficial` | 1.22.0, 2026-10-01 | iamnbutler, auto-synced every 6h | ([crates.io](https://crates.io/crates/gpui-unofficial)) |
| `gpui-ce` | 0.2.2 (2026-08-28); 0.3.2/0.3.3 yanked 2026-08-28 | community fork | Diverging API; GitComet git-pins a fork of it ([runner#733](https://github.com/yicheng47/runner/issues/733), [GitComet Cargo.toml](https://github.com/Auto-Explore/GitComet/blob/main/Cargo.toml)) |

- Zed's position (jmg-duarte, 2026-03-17): "I don't think the team has the resources to extract and maintain GPUI as a standalone library outside of Zed's purposes" ([discussion #30515](https://github.com/zed-industries/zed/discussions/30515)).
- Docs: [gpui.rs](https://www.gpui.rs/) and in-repo examples (about 45 under `crates/gpui/examples`). The site says "More docs & examples can be found throughout Zed's crates". Expect to learn by reading Zed's source.

**(5) Maintenance.** Very active. gpui commits land daily (e.g. "gpui: Make test-support safe to enable in every build", 2026-10-01). The platform layer was recently split into `gpui_platform`, `gpui_linux`, `gpui_macos`, `gpui_apple`, `gpui_wgpu`, `gpui_web`, and others ([crates dir](https://github.com/zed-industries/zed/tree/main/crates)). That split is proof the API is still moving. Zed itself releases weekly (v1.22.0 on 2026-09-30, [releases](https://github.com/zed-industries/zed/releases)).

### egui

**(1) Custom painting.**
- `epaint::CubicBezierShape` has `points: [Pos2; 4]`, `closed`, `fill`, and `stroke` ([docs.rs](https://docs.rs/epaint/latest/epaint/struct.CubicBezierShape.html)).
- But `PathShape` documents: "Fill is only supported for convex polygons" ([docs.rs](https://docs.rs/epaint/latest/epaint/struct.PathShape.html)). A Connector band is non-convex.
- Workaround: sample both curves and emit a triangle-strip `Mesh` by hand, about 30 lines. This is a known, bounded cost.

**(2) Large text.**
- `TextFormat` has `color` and `background: Color32` per section of a `LayoutJob` ([docs.rs](https://docs.rs/egui/0.36.2/egui/text/struct.TextFormat.html)). That covers syntax colors and Inline Change backgrounds.
- `ScrollArea::show_rows`: "Efficiently show only the visible part of a large number of rows" ([docs.rs](https://docs.rs/egui/0.36.2/egui/containers/scroll_area/struct.ScrollArea.html)).
- Caveats:
  - Immediate mode re-runs the UI every frame, so laid-out text must be cached.
  - Scroll jitter has been reported above about 2M rows ([#1391](https://github.com/emilk/egui/issues/1391)). That is far beyond 100k.
  - A "many Galleys" FPS collapse (<10 FPS) was reported and fixed in Aug 2026 ([#8419](https://github.com/emilk/egui/issues/8419)).
- Existence proof: [large-text-viewer](https://github.com/acejarvis/large-text-viewer) uses `show_rows` with highlights and line numbers for >10GB files. Rerun ships on egui.
- egui has no editor-grade text stack comparable to Zed's.

**(3) Platforms.** eframe runs on winit/wgpu or glow, on Linux (Wayland/X11) and macOS ([repo](https://github.com/emilk/egui)). Open Hyprland-specific reports:
- "Keyboard input completely ignored on Hyprland (Wayland)" with eframe 0.34, Hyprland 0.53.3. Opened 2026-06-22, still open ([#8249](https://github.com/emilk/egui/issues/8249)).
- SIGSEGV at exit on Hyprland + NVIDIA, open since 2024 ([#3962](https://github.com/emilk/egui/issues/3962)).

The spike must check keyboard input on the target machine.

**(4) Standalone.** First-class crates.io library: `egui`/`eframe` 0.36.2 (2026-09-08), about 24.9M downloads ([crates.io](https://crates.io/crates/egui)), plus docs.rs.

**(5) Stability.** Pre-1.0 with breaking minor versions about every 1–2 months: 0.34.3 (2026-05-27), 0.35.0 (2026-06-25), 0.36.0 (2026-08-05) ([crates.io versions](https://crates.io/crates/egui/versions)). Commits are daily ([repo](https://github.com/emilk/egui/commits/main)).

### iced

**(1) Custom painting.** The `canvas` path `Builder` has `bezier_curve_to` and `quadratic_curve_to`, lyon-backed with fill support ([builder.rs @0.14.0](https://github.com/iced-rs/iced/blob/0.14.0/graphics/src/geometry/path/builder.rs)).

**(2) Large text.**
- `text::Span` has `color` and `highlight: Option<Highlight{background, border}>` ([core/src/text.rs @0.14.0](https://github.com/iced-rs/iced/blob/0.14.0/core/src/text.rs)). The styling model fits.
- But **upstream iced has no virtualized list widget**: `widget/src/` on master has `lazy.rs`, which is memoization, not virtualization, and no `list.rs` ([widget dir](https://github.com/iced-rs/iced/tree/master/widget/src)). Upstream has no commit history for that file. A virtualized `List` exists in System76's fork ([pop-os/iced list.rs](https://github.com/pop-os/iced/blob/master/widget/src/list.rs)). An issue filed on upstream against that `List` reports a lag: all rows are re-measured whenever the width changes ([#3429](https://github.com/iced-rs/iced/issues/3429)).
- Text performance issues on record:
  - Scrolling large text degrades with total size ([#897](https://github.com/iced-rs/iced/issues/897)).
  - `text_editor` uses 20–60% CPU on simple actions ([#2477](https://github.com/iced-rs/iced/issues/2477)).
  - The tiny-skia dense-text clip regression ([#3368](https://github.com/iced-rs/iced/issues/3368)).
- We would write a custom line-virtualizing widget.

**(3) Platforms.** The strongest Wayland pedigree of any toolkit here. The COSMIC desktop, built on iced via libcosmic, has been stable since Epoch 1 (2025-12-11), with point releases through 2026 ([LWN](https://lwn.net/Articles/1050268/), [Wikipedia](https://en.wikipedia.org/wiki/COSMIC_desktop)). COSMIC itself uses a fork. macOS is supported via winit ([repo](https://github.com/iced-rs/iced)).

**(4) Standalone.** crates.io `iced` 0.14.0, about 2.86M downloads ([crates.io](https://crates.io/crates/iced)).

**(5) Stability.**
- Releases are infrequent: 0.13.0 (2024-09-18), then 0.14.0 (2025-12-07), and **none in the ~10 months since**.
- Master is `0.15.0-dev` with milestone 0.15 at 101 closed / 11 open ([milestones](https://github.com/iced-rs/iced/milestones)).
- Master git-pins its own forks of `winit` and `cosmic-text` ([Cargo.toml](https://github.com/iced-rs/iced/blob/master/Cargo.toml)).
- Active: 100+ commits since 2026-04.

**Why not the runner-up:** it is in the middle on everything, its large-text story is the weakest of the three Rust candidates, and its release cadence is slow.

### Tauri (web frontend)

- **(1)/(2)** The web side is the easiest. SVG paths for Connectors are trivial. JS editors (CodeMirror 6, Monaco) already virtualize lines and do per-span decoration. GitButler is a git client with diff views on Tauri + Svelte ([repo](https://github.com/gitbutlerapp/gitbutler)).
- **(3) Linux is the blocker.** Tauri's own docs: "On Linux, Tauri renders through WebKitGTK. On some setups, most often NVIDIA GPUs, WebKitGTK and the graphics driver disagree". Listed symptoms include blank windows, flicker on resize, and "Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display". Workarounds are env vars that trade away GPU compositing ([Tauri Linux graphics](https://v2.tauri.app/develop/debug/linux-graphics/)). A WebKitGTK 2.52.6 regression produced a blank or hung window ([example report](https://github.com/Snehal1112/rocket/issues/17)).
- A Chromium (CEF) runtime is only in **3.0.0-alpha.4 (2026-10-01)** ([crates.io](https://crates.io/crates/tauri), [releases](https://github.com/tauri-apps/tauri/releases)).
- On macOS (WKWebView) it is fine.
- **(4)/(5)** Excellent: 2.12.1 stable (2026-09-30), about 33.5M downloads ([crates.io](https://crates.io/crates/tauri)).
- Also adds a second language and runtime, plus IPC for 100k-line payloads.
- **Rejected for the Hyprland target.** Revisit when the CEF runtime is stable.

### Slint

- **(1)** `Path` supports `CubicTo` with fill and `nonzero`/`evenodd` fill rules ([docs](https://docs.slint.dev/latest/docs/slint/reference/elements/path/)).
- **(2)** `ListView` instantiates only visible elements ([docs](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/listview/)).
- **The blocker:** the rich-text element `StyledText` takes CommonMark plus a few HTML tags. Colors are set via `<font color>`, and **per-span background highlight is not supported** ([StyledText docs](https://docs.slint.dev/latest/docs/slint/reference/elements/styledtext/)). Inline Change backgrounds would need overlay rectangles positioned from our own text metrics. Feeding syntax colors through markup strings for 100k lines is a poor fit.
- **(4)/(5)** The most stable API here: 1.18.1 (2026-09-21), semver 1.x ([crates.io](https://crates.io/crates/slint), [1.18 blog](https://slint.dev/blog/slint-1.18-released)). Licenses are GPLv3, royalty-free with attribution for desktop, or commercial ([LICENSE.md](https://github.com/slint-ui/slint/blob/master/LICENSE.md)).

### Floem, Xilem, Makepad (briefly)

- **Floem** powers the Lapce editor. But crates.io is still 0.2.0 (2024-11-14), Lapce depends on it by git ([lapce Cargo.toml](https://github.com/lapce/lapce/blob/master/Cargo.toml)), and the repo saw 5 commits from 2026-04 to 2026-10 ([commits](https://github.com/lapce/floem/commits/main)). README: "We will make occasional breaking changes" ([README](https://github.com/lapce/floem)). It is the same standalone problem as gpui, with far less momentum.
- **Xilem/Masonry** describes itself as "An experimental Rust architecture for reactive UI" ([README](https://github.com/linebender/xilem)). 0.4.0 shipped 2025-10-29. Its building blocks (Vello 0.10, Parley 0.11) are healthy ([crates.io/vello](https://crates.io/crates/vello), [crates.io/parley](https://crates.io/crates/parley)), but the toolkit is not ready.
- **Makepad** is active (dev branch daily, including Wayland fixes on 2026-09-30, [commits](https://github.com/makepad/makepad/commits/dev)). But crates.io `makepad-widgets` 1.0.0 dates from 2025-05-13. It uses its own live DSL and shader language, and is now pitched as "an AI-accelerated application and game development environment" ([README](https://github.com/makepad/makepad)). Niche fit for us.

## Open risks

1. **Criterion (2) is unmeasured for every candidate.** All large-text evidence is vendor claims ("200K lines", "opens Chromium in about a second") or issue reports. The <300ms open for 100k lines must be verified in the spike. That includes diff computation and syntax highlighting, which are toolkit-independent.
2. **gpui supply chain.** If we git-pin Zed, upgrades are manual and API breaks are expected ("often breaking changes"). The platform crate split in 2026 shows how much churn to expect. Third-party republishers each have a single owner (`gpui-pre`: huacnlee; `gpui-unofficial`: iamnbutler), and `gpui-ce` already yanked a release line. Mitigation: pin an exact Zed tag and keep gpui usage behind a thin rendering module so a version bump touches a few files.
3. **gpui docs.** They are thin outside the examples. Expect to read Zed source, and don't copy Zed's GPL `editor` code.
4. **Wayland fractional scaling** is a live issue class in gpui ([#65032](https://github.com/zed-industries/zed/issues/65032), [discussion #43869](https://github.com/zed-industries/zed/discussions/43869)). Test on the actual Hyprland setup at the user's scale factor.
5. **egui fallback cost.** Connector meshes must be tessellated by hand, galleys must be cached, and the breaking-minor cadence of about every 1–2 months means regular upgrade work. There is also an open "keyboard input ignored on Hyprland" report ([#8249](https://github.com/emilk/egui/issues/8249)).
6. **macOS evidence is thin for all toolkits.** It rests on each project's README or platform list, plus Zed, Rerun, and GitButler shipping there. Nothing here was checked specifically for diff-viewer workloads.
