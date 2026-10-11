# Can Zed's `editor` crate be reused outside Zed?

Research for ticket `10-zed-editor-reuse-research`. Everything below is checked against Zed tag
`v1.22.0` (commit `76659a55a8c1`, 2026-09-30), the same tag this project pins gpui to.
Researched 2026-10-01.

## Recommendation

**(a) Chunk-only operations** (move a Change Block from one Side to the other, accept/reject in a
merge result): **don't use an editor component at all.** The viewer already owns the line model,
the Side renderer and the Connectors (see the two-pane connector prototype). A chunk move is a text
splice on the target Side, a re-diff and a redraw, plus an undo stack of splices. That is a few
hundred lines of our own Apache/MIT code. Zed's `editor`, gpui-component and a hand-rolled editor
all cost more than that.

**(b) Free typing** (edit a Side, or the 3-way merge result pane, like a real editor):

| Option | Licence | gpui coupling | Row backgrounds / filler rows / inline buttons | Cost |
|---|---|---|---|---|
| **Zed `editor`** | Whole app becomes GPL-3.0-or-later | Same repo and tag as our gpui pin, so a perfect fit | Yes: `highlight_rows`, `insert_blocks` (spacers, buttons), `register_addon` | +354 crates, about 3 min extra release build, binary 25 MB to 117 MB, about 1,050 commits a year of churn |
| **gpui-component `Editor`** | Apache-2.0 | Pins `gpui-pre =0.3.7` (crates.io snapshot of zed@1a28cff), **not** our git tag. We'd have to switch gpui source or fork it | Byte-range text and Fill/Frame decorations, folding, line numbers, scroll get/set. **No block/filler-row API found** | Moderate. Its 200K-line, tree-sitter and LSP editor comes along |
| **Own minimal editor** | Ours (Apache/MIT) | None | Whatever we build | Most code: IME/input handler, cursor/selection, undo, virtualised rendering |

**Recommendation:** ship (a) with our own code now. For (b), **prefer reusing Zed's `editor` under
GPL-3.0-or-later**, but only if the project accepts the GPL licence (open question in `map.md`). It is the only option that
gives real editing, undo, filler-row blocks, row highlights and inline action buttons on our exact
gpui pin. I verified it runs standalone (below). If the project must stay permissive, build our
own minimal editor on top of the (a) renderer: typing inside an already-virtualised Side, using
gpui's `EntityInputHandler`. Don't adopt gpui-component just for this, because it forces a
gpui-source switch across the whole app.

## 1. Dependencies of `crates/editor` at v1.22.0

Source: <https://raw.githubusercontent.com/zed-industries/zed/refs/tags/v1.22.0/crates/editor/Cargo.toml>.
`license = "GPL-3.0-or-later"`; `publish.workspace = true` resolves to `publish = false` (root `[workspace.package]`). These crates are not on crates.io
under these names, so they're consumed as git dependencies.

Direct Zed-internal dependencies (38): `assets`, `breadcrumbs`, `buffer_diff`, `client`, `clock`,
`collections`, `dap`, `db`, `edit_prediction_types`, `feature_flags`, `file_icons`, `fs`, `fuzzy`,
`git`, `gpui`, `language`, `language_detection`, `lsp`, `markdown`, `menu`, `multi_buffer`,
`project`, `rope`, `rpc`, `settings`, `snippet`, `sum_tree`, `task`, `telemetry`, `text`, `theme`,
`theme_settings`, `ui`, `ui_input`, `util`, `vim_mode_setting`, `workspace`, `zed_actions`
(+ `zlog`, `ztracing`). `project`, `workspace`, `lsp`, `dap` and `client` are **hard, non-optional**
dependencies: they get compiled and linked even when no Project is created at runtime.

Transitive graph, measured on Linux:

```
# shallow clone of the tag
git clone --depth 1 --branch v1.22.0 https://github.com/zed-industries/zed
cargo metadata --format-version 1 --locked > meta.json
cargo tree -p editor -e normal,build --prefix none --target x86_64-unknown-linux-gnu --offline
cargo tree -p gpui -p gpui_platform ...   # same flags, baseline
```

| Measure | Count |
|---|---|
| Unique packages in `editor`'s normal+build graph | **659** |
| …of which Zed workspace crates | **98** (97 besides `editor`) |
| Packages *beyond* a plain `gpui` + `gpui_platform` app (351) | **354** (83 Zed crates + 271 third-party) |

Licences of the 97 Zed crates pulled (from `cargo metadata` `license` field):

- **GPL-3.0-or-later: 72.** These are `agent_settings askpass assets breadcrumbs buffer_diff client clock component
  context_server credentials_provider dap db edit_prediction_types env_var extension feature_flags
  feature_flags_macros file_icons fs fuzzy fuzzy_nucleo git git_hosting_providers icons language
  language_core* language_detection language_model language_model_core lsp markdown menu
  mermaid_render migrator multi_buffer net node_runtime paths prettier project proto
  release_channel remote rope rpc session settings settings_content settings_json settings_macros
  snippet snippet_provider sqlez sqlez_macros syntax_theme task telemetry telemetry_events terminal
  text theme theme_settings ui ui_input ui_macros vim_mode_setting workspace worktree zed_actions
  zed_credentials_provider zed_env_vars zeta_prompt`.
  (\* `language_core` has no `license` field. It ships `LICENSE-GPL -> ../../LICENSE-GPL`.)
- **Apache-2.0: 25.** These are `cloud_api_client cloud_api_types cloud_llm_client collections derive_refineable gpui
  gpui_macros gpui_shared_string gpui_tokio gpui_util http_client http_client_tls
  oauth_callback_server path perf proxy_handshake refineable scheduler sum_tree util util_macros
  watch zlog ztracing ztracing_macro`.
- **AGPL: 0.** No `Cargo.toml` in `crates/` mentions AGPL, and the repo root has only
  `LICENSE-GPL` and `LICENSE-APACHE`.

Heavy third-party pulls and where they come from (`cargo tree -p editor -i <crate>`):

- `wasmtime` 48 + `cranelift-*`: via `tree-sitter` (wasm feature), pulled in by `language`.
- `alacritty_terminal` (Zed fork): via `terminal`, pulled in by `project`.
- `libsqlite3-sys`: via `sqlez`, pulled in by `db`, which is a direct `editor` dependency.
- `rustls`, `tokio`, `async-tungstenite`: via `client`, `http_client_tls`, and from there `project`, `workspace`, `dap`.

## 2. Standalone construction: **yes, verified**

Constructors (<https://github.com/zed-industries/zed/blob/v1.22.0/crates/editor/src/editor.rs#L1800-L1919>):
`Editor::single_line`, `multi_line`, `auto_height(min,max)`, `auto_height_unbounded`,
`for_buffer(buffer, project: Option<Entity<Project>>, …)`, `for_multibuffer(mb, Option<Project>, …)`,
`new(mode, mb, Option<Project>, …)`. **`Project` is `Option` everywhere.** A single file becomes
`MultiBuffer::singleton(Buffer::local(text))`.

Global state: `editor::init` (editor.rs#L360) sets two globals, registers the Editor with
`workspace` registries and observes `Workspace` creation. It needs no `AppState` unless a
`workspace::NewFile`/`NewWindow` action actually fires. Zed's own unit-test setup
(`editor_tests.rs#L41644` `init_test`) shows the minimum: a `SettingsStore`,
`theme_settings::init(LoadThemes::JustBase)`, `release_channel::init`, `editor::init`. No Project,
no Workspace. (`EditorTestContext::new` does build a `Project::test`, but the plain `Editor::new(…,
None, …)` path doesn't need one.)

**Probe:** I built a throwaway binary inside the cloned workspace (`crates/probe_editor`, not in this
repo). It contains only:

```rust
assets::Assets.load_fonts(cx)?;
settings::init(cx);
theme_settings::init(theme::LoadThemes::JustBase, cx);
release_channel::init(semver::Version::new(0, 0, 0), cx);
editor::init(cx);
// keymap: see below
cx.open_window(WindowOptions::default(), |window, cx| {
    theme_settings::setup_ui_font(window, cx);
    cx.new(|cx| {
        let buffer = cx.new(|cx| Buffer::local(text, cx));
        let mb = cx.new(|cx| MultiBuffer::singleton(buffer, cx));
        Editor::new(EditorMode::full(), mb, None, window, cx)
    })
})
```

Results (Linux/Wayland, Hyprland):
- The window opens and renders the file with line numbers, indent guides, current-line highlight
  and minimap. There's no panic and no missing-global error (screenshot of the `--smoke` run after undo, with the `highlight_rows` red row: [`zed-editor-probe.png`](zed-editor-probe.png)).
- A `--smoke` run inserts at `Point(0,0)` with `editor.edit`, then `editor.undo`, then
  `highlight_rows::<T>`. It printed
  `SMOKE OK edit="moved\nleft\nright\n" undo="left\nright\n"`.
- **Gotcha, keymap:** `KeymapFile::load_asset(DEFAULT_KEYMAP_PATH)` **fails**:
  `didn't find an action named "picker::ConfirmInput"`. The default keymap references actions from
  crates we didn't link. The workaround is `KeymapFile::load(&default_keymap(), cx)` and keeping the
  `SomeFailedToLoad { key_bindings, .. }`. That loaded 579 bindings, though I didn't test them with key presses. Better still, ship our own
  small keymap.
- **Not included without more setup:** syntax highlighting needs a `LanguageRegistry` plus grammars
  (`languages` crate or our own). LSP, project search and git gutter need a `Project`.

Other references: `crates/component_preview/examples/component_preview.rs` boots the *full* stack
(Client, UserStore, NodeRuntime, Session, `db::AppDatabase`, `Project::local`, `Workspace`). It's
the heavyweight pattern, not the minimum.

## 3. Third-party apps embedding Zed's editor crates

Search: `gh api search/code -f q='zed-industries/zed editor filename:Cargo.toml'` (81 hits, filtered
by hand because "editor" matches any text) and `…multi_buffer filename:Cargo.toml` (4 hits).

| Repo | Licence | How | Notes |
|---|---|---|---|
| [neguse/zec](https://github.com/neguse/zec) | GPL-3.0 | `editor`, `project`, `workspace`, `languages`, … as git deps pinned to a rev. "There is no fork" | A terminal editor running Zed's `Editor` headless. Init in `src/zed/runtime.rs`: `db::AppDatabase`, `gpui_tokio::init`, `settings::init`, `theme_settings::init(All)`, `editor::init`, `trusted_worktrees::init`, `Client::production`, `Project::init`, `Fs` global, `LanguageRegistry` and `languages::init`. Its roadmap notes the plain Buffer/Editor path "carries a 100,000-line file" |
| [thinkter/zmux](https://github.com/thinkter/zmux) | GPL-3.0 | ~30 Zed crates as git deps pinned to a rev, including `editor`, `terminal_view`, `workspace` | Keeps Zed's whole `Workspace`. **Vendors and patches** `terminal` and `workspace` (`docs/architecture.md` "Vendored Zed crates"). CI checks that vendored crates carry LICENSE files |
| [buildzaku/zaku](https://github.com/buildzaku/zaku) | AGPL-3.0 | **Vendored, trimmed copies** of `editor`, `multi_buffer`, `language`, `project`, … under `crates/` | An API client. Its `editor` is cut down to about 15 deps. This is the fork-and-strip route |
| [azw413/Glass](https://github.com/azw413/Glass) | GPL-3.0 | `text`, `rope`, `language`, `multi_buffer` only (no `editor`) | Uses Zed's buffer layer under its own UI |
| [reviu-dev/reviu](https://github.com/reviu-dev/reviu) | FSL-1.1-ALv2 | Own `editor` crate on **gpui-component** plus forked gpui | A code-review / diff app. Avoided Zed's GPL editor |
| [kylekz/diff](https://github.com/kylekz/diff) | MIT | gpui + gpui-component | A small diff tool on gpui-component |

Pattern: everyone who embeds Zed's `editor` is GPL/AGPL, pins an exact Zed rev, and moves only
when they bump that rev. The bigger ones end up vendoring or patching crates.

## 4. Zed's own merge-conflict UI and side-by-side diff

**Conflict resolution**, `crates/git_ui/src/conflict_view.rs` (git_ui is GPL-3.0-or-later):
- It works on **one buffer with `<<<<<<<`/`=======`/`>>>>>>>` markers**. `project::git_store::ConflictSet`
  (`crates/project/src/git_store/conflict_set.rs`) parses the regions.
- It's attached to an ordinary `Editor` via `editor.register_addon(ConflictAddon)`. Ours/theirs/marker
  rows are tinted with `highlight_rows::<ConflictsOurs/Theirs/…>`, and a block above each conflict is
  added with `insert_blocks(BlockProperties{…})`. That block renders buttons
  `"Use {ours_branch}"`, `"Use {theirs_branch}"`, `"Use Both"` (L335-L390).
  `resolve_conflict` (L485) rewrites the buffer, then removes the highlights and block.
- It is **not a 3-pane merge**: there's no base/left/right/result layout. It also needs a
  `Project`/`GitStore` to get the `ConflictSet`. Reuse would be at the level of the pattern only,
  though the pattern (addon, row highlights, inline button blocks) is exactly what a merge result
  pane needs.

**Side-by-side diff**, `crates/editor/src/split.rs` (`SplittableEditor`, ~6.4k lines) plus
`split_editor_view.rs`:
- `SplittableEditor::new(style, rhs_multibuffer, project: Entity<Project>, workspace:
  Entity<Workspace>, …)`. Project and Workspace are **required**.
- The RHS is the editable buffer. The LHS is built in `split()` as
  `MultiBuffer::without_headers(Capability::ReadOnly)` over the `BufferDiff` **base text**
  (L706-L752). So it's buffer vs base with one editable side, kept aligned by an internal
  `Companion` display-map link (`display_map.rs` `Companion`, `CompanionExcerptPatch`).
- Two **independently editable** Sides with chunk moves both ways (our use case) aren't what it
  models. With Zed's editor we would build two plain `Editor`s and do our own alignment, using
  `insert_blocks` spacers as Filler Rows. `Companion` is `pub(crate)`.

## 5. Costs

Measured on 32 cores and 62 GB RAM, profile `probe` = `release` with `lto=false`,
`codegen-units=16`, `debug=false`, `strip=true`, shared `CARGO_TARGET_DIR`:

| Build | Wall time | Units compiled | Stripped binary |
|---|---|---|---|
| gpui + gpui_platform hello window (cold) | 49 s | 456 | **25.4 MB** |
| + `editor` probe (incremental on top) | **+193 s** | +607 | **117.1 MB** |

`cargo metadata` on the cold clone took 3m42s, mostly downloads. Zed's real `release` profile
(`lto="thin"`, `codegen-units=1`) would be slower and somewhat smaller. Target dir: 4.1 GB.

**API churn** (`gh api repos/zed-industries/zed/commits?path=…&sha=v1.22.0&since=2025-10-01`):
**1,051 commits** touched `crates/editor` in the 12 months before the tag, and **483** touched
`editor.rs` alone. `crates/editor/src` is ~182k lines. The crates are internal (`publish = false`,
no semver promise), so expect breakage on every gpui bump. On the plus side, the editor and
gpui always move together on the same tag, so there's never a version-skew problem.

## 6. Alternatives (Apache-2.0)

**gpui-component** ([longbridge/gpui-component](https://github.com/longbridge/gpui-component),
crates.io `gpui-component` 0.7.0, 2026-09-28, Apache-2.0):
- `Editor`/`EditorState` (`crates/base/src/input/editor/`, `crates/component/src/input/editor.rs`).
  README: "Stable performance at 200K lines with Tree-sitter highlighting and LSP diagnostics,
  completion, and hover". Ropey-backed. It has `set_value`/`insert`/`replace`, `readonly`,
  `line_number`, `folding`, `soft_wrap`, `scroll_offset`/`set_scroll_offset` (usable for
  Scroll Mode sync), `visible_row_range`, an undo manager with nested transactions
  (`base/undo_manager.rs`), and decorations `TextDecoration` (`HighlightStyle` over a byte range) and
  `RangeDecoration` (`Fill`/`Frame`) (`editor/decorations.rs`).
- **Gaps for us:** I found no API for inserting non-text block rows (Filler Rows, inline
  buttons between lines). The display map is only Wrap→Fold (`display_map/README.md`). Full-row
  backgrounds would have to be approximated with `RangeDecoration::Fill`.
- **The blocker is gpui coupling.** `Cargo.toml` L68 has
  `gpui = { package = "gpui-pre", version = "=0.3.7" }`. crates.io describes it as a "snapshot of
  zed@1a28cff". `gh api repos/zed-industries/zed/compare/v1.22.0...1a28cff` gives
  *diverged*, 47 ahead / 10 behind, merge base 2026-09-23. It's a different crate name, so its
  types aren't our gpui's types. Adopting it means moving the whole app to `gpui-pre`, or forking
  it the way reviu and tty7 do with `[patch]`.

**gpui's own examples** (Apache-2.0, in `crates/gpui/examples/`):
- `input.rs` (784 lines) is a single-line `TextInput`. It turns `\n` into a space and implements
  `EntityInputHandler` (IME, selection, clipboard). There's no undo.
- `view_example/example_editor.rs` (549 lines) plus `example_text_area.rs` is a multi-line text area
  over an `Entity<String>`. It shapes every line each frame, so no virtualisation, and has no undo.
- Together they're a reasonable seed for "light typing" inside our own Side renderer. Our renderer
  already virtualises rows, so we'd add the input handler, a cursor/selection, and an undo stack of
  splices shared with the chunk-move undo.

**Own minimal editor, feasibility:** the hard parts are already in gpui (`EntityInputHandler`, text
shaping, `ShapedLine::x_for_index`/`index_for_x`). Chunk operations plus single-cursor typing, selection,
clipboard and undo over a rope (e.g. `ropey`, or Zed's Apache-licensed `sum_tree`) is a
bounded job. Multi-cursor, soft wrap, IME edge cases and large-file incremental re-diff are where the
cost grows. Here "own" means the project licence stays free.

## Risks

- **Licence:** embedding `editor` makes the distributed binary GPL-3.0-or-later. The Apache-2.0
  engine crate stays Apache (one-way compatible), but the app can't be relicensed permissively later
  without removing `editor`.
- **Weight:** you get `project`, `lsp`, `dap`, `client`, `terminal`, sqlite and wasmtime even if they're never used.
  The only way out is vendoring and stripping (zaku's route), which becomes a fork to maintain.
- **Churn:** about 20 editor commits a week, and internal APIs (`insert_blocks`, `BlockProperties`,
  `highlight_rows`) change shape without notice. Pin the tag and budget a fix-up on every bump.
- **Hidden runtime coupling:** default keymap failures (above), and `workspace::NewFile` handlers
  that assume `AppState`. Keep our own keymap and don't bind workspace actions.
- **Zed's diff/conflict UIs aren't reusable as-is.** Both require `Project`/`Workspace`, and the
  split view is one-side-editable.

## Appendix: reproducing the standalone probe

In a shallow clone of `v1.22.0`, add `"crates/probe_editor"` to `[workspace] members` and add this
profile to the root `Cargo.toml`:

```toml
[profile.probe]
inherits = "release"
debug = false
strip = true
lto = false
codegen-units = 16
```

`crates/probe_editor/Cargo.toml`:

```toml
[package]
name = "probe_editor"
version = "0.1.0"
edition.workspace = true
publish = false
license = "GPL-3.0-or-later"

[dependencies]
assets.workspace = true
editor.workspace = true
gpui.workspace = true
gpui_platform = { workspace = true, features = ["font-kit", "wayland", "x11"] }
language.workspace = true
multi_buffer.workspace = true
release_channel.workspace = true
semver.workspace = true
settings.workspace = true
theme.workspace = true
theme_settings.workspace = true
```

`crates/probe_editor/src/main.rs`:

```rust
use editor::{Editor, EditorMode, RowHighlightOptions};
use gpui::{AppContext as _, WindowOptions};
use language::{Buffer, Point};
use multi_buffer::MultiBuffer;

struct ProbeHighlight;

fn main() {
    let arg = std::env::args().nth(1);
    let smoke = arg.as_deref() == Some("--smoke");
    gpui_platform::application().with_assets(assets::Assets).run(move |cx| {
        assets::Assets.load_fonts(cx).expect("fonts");
        settings::init(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        editor::init(cx);
        match settings::KeymapFile::load_asset(settings::DEFAULT_KEYMAP_PATH, None, cx) {
            Ok(bindings) => cx.bind_keys(bindings),
            Err(e) => {
                eprintln!("KEYMAP ERR {e}");
                // Keep the bindings whose actions are linked into this binary.
                if let settings::KeymapFileLoadResult::SomeFailedToLoad { key_bindings, .. } =
                    settings::KeymapFile::load(&settings::default_keymap(), cx)
                {
                    eprintln!("KEYMAP PARTIAL {} bindings", key_bindings.len());
                    cx.bind_keys(key_bindings);
                }
            }
        }
        cx.open_window(WindowOptions::default(), |window, cx| {
            theme_settings::setup_ui_font(window, cx);
            cx.new(|cx| {
                let text = match arg.as_deref() {
                    Some(p) if !smoke => std::fs::read_to_string(p).unwrap(),
                    _ => "left\nright\n".into(),
                };
                let buffer = cx.new(|cx| Buffer::local(text, cx));
                let mb = cx.new(|cx| MultiBuffer::singleton(buffer, cx));
                let mut editor = Editor::new(EditorMode::full(), mb, None, window, cx);
                if smoke {
                    editor.edit([(Point::new(0, 0)..Point::new(0, 0), "moved\n")], cx);
                    let after_edit = editor.text(cx);
                    editor.undo(&editor::actions::Undo, window, cx);
                    let after_undo = editor.text(cx);
                    let snapshot = editor.buffer().read(cx).snapshot(cx);
                    let range = snapshot.anchor_before(Point::new(0, 0))
                        ..snapshot.anchor_after(Point::new(1, 0));
                    editor.highlight_rows::<ProbeHighlight>(
                        range, |_| gpui::red(), RowHighlightOptions::default(), cx,
                    );
                    assert_eq!(after_edit, "moved\nleft\nright\n");
                    assert_eq!(after_undo, "left\nright\n");
                    eprintln!("SMOKE OK edit={after_edit:?} undo={after_undo:?}");
                }
                editor
            })
        })
        .unwrap();
        cx.activate(true);
    });
}
```

Build and run:

```sh
CARGO_TARGET_DIR=$TMPDIR/zed/target cargo build --offline --profile probe -p probe_editor
$TMPDIR/zed/target/probe/probe_editor --smoke        # prints KEYMAP ... and SMOKE OK ...
$TMPDIR/zed/target/probe/probe_editor some/file.rs   # opens an editable window
```

The `probe_gpui` baseline is a gpui window containing a single `div`, built with the same
`gpui_platform` features.
