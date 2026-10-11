# Syntax highlighting engine: tree-sitter vs syntect

Research for ticket `04-syntax-highlighting-research`. Researched 2026-10-01.
Terms (**Side**, **Comparison**, **Change Block**, **Inline Change**) are as defined in `CONTEXT.md`.

## Recommendation

**Use tree-sitter.** Start with the official `tree-sitter` + `tree-sitter-highlight` crates and the
per-language grammar crates, covering a curated set of about 15–30 languages. Files in any other
language render as plain text.

Why:

- **Speed.** On 100k-line files it was **about 3.6–6× faster** than syntect with Oniguruma
  (0.32–0.63 s vs 1.2–3.7 s) and **about 13–21× faster** than syntect's pure-Rust `fancy-regex` backend
  (measured, see [Benchmark](#benchmark)).
- **Edits.** Re-parsing after a one-line edit took 3–10 ms.
- **Viewport-only work.** tree-sitter can highlight just the visible byte range of an
  already-parsed tree. syntect has to scan every line from the top of the file.
- **AST-aware diff later.** That feature needs a tree-sitter parse tree. difftastic is built on
  tree-sitter grammars and highlight queries. With syntect we would need a second, separate
  parsing stack.
- **Inline Change composition.** Both engines output (byte range → style) runs, so they combine
  with Inline Change backgrounds the same way. This does not decide between them.

**What it costs:** binary size. Parser tables are static data, averaging about 1.5 MB per grammar.
JavaScript adds 0.4 MB; C++ and C# add far more. 30 grammars come to 48.5 MB stripped (5.0 MB gzipped). syntect plus two-face covers 220 syntaxes
in 2.3 MB. Keep the grammar list curated and never enable "all languages" features.

**Don't build:** a syntect fallback for the long tail of languages. If users ask for more
coverage than the curated set, two-face adds 220 syntaxes for about 2.3 MB.

## Comparison

| | **tree-sitter** (+ `tree-sitter-highlight`, grammar crates / arborium) | **syntect** (+ two-face = bat's set) |
|---|---|---|
| Latest version (2026-10-01) | tree-sitter / -highlight 0.27.0 (2026-08-30); arborium 2.18.2 ([crates.io](https://crates.io/crates/tree-sitter)) | syntect 5.3.0 (2025-09-27); two-face 0.5.2+bat-0.26.1 ([crates.io](https://crates.io/crates/syntect)) |
| Out-of-box languages | One crate per grammar. arborium bundles 112 behind `lang-*` features; difftastic ships 65; tree-sitter-language-pack 371 (downloaded at runtime) | syntect defaults: 75 syntaxes (no TypeScript, TOML, Kotlin, Swift, Dart…); two-face: 220 (onig) / 213 (fancy). Measured |
| Rust, 99,838 lines, parse + highlight | **322–342 ms** (arborium 608–623 ms) | 1,225–1,250 ms (onig), 4,318 ms (fancy) |
| JS, 100,000 lines, parse + highlight | **614–629 ms** (arborium 578–583 ms) | 3,601–3,728 ms (onig), 13,035 ms (fancy); two-face 4.9 s (onig) / 15.7 s (fancy) |
| Incremental re-parse after 1 edit | 3–10 ms | n/a: re-scan from the nearest cached line state |
| Highlight only the visible range | Yes, with our own `QueryCursor` loop (`set_byte_range`) over the parsed tree. `tree-sitter-highlight` itself can't | No. State is sequential, so cache state every ~1000 lines |
| Output | `HighlightEvent::{Source{start,end}, HighlightStart(idx), HighlightEnd}`. Capture names are mapped to a theme by us | `Vec<(Style{fg,bg,font_style}, &str)>` per line, themed by `.tmTheme` |
| Stripped binary size (measured) | runtime + Rust + JS: 3.96 MB; Rust only: 3.52 MB; 30 langs (arborium): 48.5 MB; 112 langs: 167 MB | defaults (75): 1.57 MB; two-face (220): 2.26 MB; empty Rust bin: 0.35 MB |
| C toolchain needed | Yes, for every grammar (`cc`). No pure-Rust path | Yes for onig (default). The `fancy-regex` backend is pure Rust but 3.5× slower |
| License | runtime MIT. Each grammar has its own license (mostly MIT). arborium MIT/Apache, but `all-languages` includes a GPL grammar | syntect MIT. Bundled grammars: Sublime custom permissive, MIT, BSD, Apache, Unlicense, WTFPL (no copyleft) |
| Serves AST-aware diff later | Yes. difftastic is built on it | No |
| Maintenance | Very active. Helix, Zed, difftastic, GitHub.com | "mostly complete … not under heavy development" |

## Findings

### 1. Language coverage

- **syntect** ships Sublime Text's default package set. In our build `SyntaxSet::load_defaults_newlines()`
  reported **75 syntaxes**. A probe found it has **no** TypeScript/TSX, TOML, Kotlin, Swift, SCSS, Zig,
  Vue, Svelte, Nix, Protobuf, Dart or Elixir. Rust, JS, Python, Go, Java, C/C++, C#, Ruby, PHP, shell,
  HTML, CSS, JSON, YAML, Markdown, SQL, XML, Lua and Scala are present. (Measured with the
  `hlbench/src/bin/cov.rs` probe, see appendix.)
- **two-face** packages bat's syntax and theme set as a versioned crate: "a bundle bat's syntaxes.bin
  and themes.bin files, but with proper versioning, dependency tracking, and feature flags"
  ([docs.rs/two-face](https://docs.rs/two-face/latest/two_face/)). It loaded **220 syntaxes** (onig)
  and **213** (fancy) in our build. Every gap listed above is filled. The fancy count is lower because
  "fancy doesn't support all of the features used by some of the syntax definitions, so some of the
  definitions are excluded", including JavaScript (Babel), Sass and PowerShell
  ([docs.rs/two-face](https://docs.rs/two-face/latest/two_face/)).
- **syntect grammar freshness.** Sublime's v2 syntax format (`version: 2`, `extends`, `meta_prepend`…)
  was tracked in [syntect#323](https://github.com/trishume/syntect/issues/323). It is now closed with most items
  merged (PRs #419, #610, #612). The one item still unimplemented is `pop: N` multi-pop. syntect describes
  itself as "mostly complete, I still maintain it and review PRs, but it's not under heavy development"
  ([github.com/trishume/syntect](https://github.com/trishume/syntect)).
- **tree-sitter** has no blessed bundle. Each grammar is its own crate with its own release cadence.
  For example, `tree-sitter-typescript` was last released 0.23.2 on 2024-11-11, while `tree-sitter-rust`
  is at 0.24.2 (2026-03-27) ([crates.io](https://crates.io/crates/tree-sitter-typescript)). The ways
  other projects bundle grammars:
  - **arborium** (bearcove) offers 112 `lang-*` features. `default = []`, so you opt in per language
    ([Cargo.toml](https://docs.rs/crate/arborium/2.18.2/source/Cargo.toml)). It provides grammars plus
    queries, an HTML/ANSI renderer, and an `advanced` module exposing raw `Span`s, and it re-exports
    `tree_sitter` ([docs.rs/arborium](https://docs.rs/arborium/2.18.2/arborium/)). The runtime is a fork,
    `arborium-tree-sitter`: "a fork of the official `tree-sitter` crate … Pinned version to ensure consistent
    behavior across all grammar crates"
    ([README](https://docs.rs/crate/arborium-tree-sitter/2.18.2/source/README.md)).
  - **difftastic** depends on grammar crates from crates.io where possible and vendors the rest
    ([adding a parser](https://difftastic.wilfred.me.uk/adding_a_parser.html)). `difft --list-languages`
    lists 65 languages (0.71.0, measured).
  - **Helix** fetches and compiles grammars at runtime into `runtime/grammars/<name>.so`, using
    `hx --grammar fetch` / `hx --grammar build`. It keeps its own Helix-specific query files under
    `runtime/queries/<name>/` ([Helix docs](https://docs.helix-editor.com/guides/adding_languages.html)).
  - **Zed** declares grammars per extension by `repository` + `rev` in `extension.toml` and compiles
    them from there ([Zed docs](https://zed.dev/docs/extensions/languages)).
  - **tree-sitter-language-pack** advertises 371 languages, with parsers "fetched on first use and cached
    locally" ([GitHub](https://github.com/xberg-io/tree-sitter-language-pack)). Downloading at runtime
    is the wrong model for an offline desktop app unless we prefetch at package time.
- **Capture names are not standardised.** Highlight names are "arbitrary". Common conventions include
  `keyword`, `function`, `type`, `string` and dotted variants such as `function.builtin`. They are mapped
  to colours by the application
  ([tree-sitter docs](https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html)). Helix and
  Zed both maintain their own query files (links above). So with tree-sitter we own a mapping from
  capture name to theme slot, and possibly patched queries. syntect instead gets this for free from
  TextMate scopes plus `.tmTheme`.

### 2. Highlight speed on a 100k-line file

Measured on the machine and versions below. Each was run twice in a release build; both runs are
shown as a range.

| Engine | Rust (99,838 lines, 3.4 MB) | JavaScript (100,000 lines, 4.9 MB) |
|---|---|---|
| syntect 5.3 defaults, onig, parse only | 1,153–1,164 ms | 3,549–3,575 ms |
| syntect 5.3 defaults, onig, parse + style | 1,225–1,250 ms | 3,601–3,728 ms |
| two-face (bat set), onig, parse + style | 1,366–1,381 ms | 4,888–4,911 ms |
| syntect defaults, fancy-regex, parse + style | 4,318 ms | 13,035 ms |
| two-face, fancy-regex, parse + style | 4,662 ms | 15,702 ms |
| tree-sitter 0.27, parse only | 154–171 ms | 210–212 ms |
| tree-sitter 0.27, incremental re-parse after inserting one line mid-file | 8.9–9.6 ms | 3.0–3.1 ms |
| tree-sitter-highlight 0.27, parse + highlight | **322–342 ms** | **614–629 ms** |
| arborium 2.18.2 `highlight_spans` | 608–623 ms | 578–583 ms |

- In throughput terms, syntect with onig runs at about 80k lines/s on Rust and 27k lines/s on JS.
  tree-sitter-highlight runs at about 300k and 160k lines/s. syntect's own README numbers fit the same
  picture: "600ms" for 9,200-line jQuery and "50,000 lines/sec" for XML on older hardware. It also notes
  that fancy-regex "currently seems to be about half the speed" of Oniguruma
  ([syntect README](https://github.com/trishume/syntect)). On this corpus we measured 3.5× slower.
- **Comparing a Side takes two highlights.** A Comparison of two 100k-line Sides costs about 2.5–7 s
  of CPU with syntect/onig, versus 0.6–1.3 s with tree-sitter. The two Sides are independent, so they
  run in parallel either way.
- **Architecture matters more than the constant factor.**
  - syntect's `ParseState` is sequential: highlighting line N requires parsing lines 0..N. The README's
    advice for editors is to cache parse states "every ~1000 lines" and re-highlight from the nearest
    cache ([syntect README](https://github.com/trishume/syntect)).
  - tree-sitter parses the whole file once (~150–210 ms here). After that,
    [`QueryCursor::set_byte_range`](https://docs.rs/tree-sitter/0.27.0/tree_sitter/struct.QueryCursor.html#method.set_byte_range)
    limits query matching to the visible lines. Edits are applied with
    [`Tree::edit`](https://docs.rs/tree-sitter/0.27.0/tree_sitter/struct.Tree.html#method.edit) plus a
    re-parse, which takes milliseconds.
  - Caveat: `tree-sitter-highlight`'s
    [`Highlighter::highlight`](https://docs.rs/tree-sitter-highlight/0.27.0/tree_sitter_highlight/struct.Highlighter.html)
    takes `source: &[u8]`, parses internally, and highlights the whole document. It does not accept an
    existing `Tree` or a range. Viewport-only highlighting, and sharing one tree with AST diff, both
    need a small highlighter of our own over `QueryCursor`. Helix (tree-house) and difftastic each
    have one; see §5.
- Both tree-sitter corpora reported `has_error() == true`. The Rust corpus is unrelated files glued
  together, and the JS corpus is cut off at line 100,000. tree-sitter highlights through error nodes,
  so this did not stop highlighting. Real Side contents normally parse cleanly.

### 3. Composing highlight spans with Inline Change backgrounds

- **Both engines produce foreground runs; an Inline Change is a background range.** Composition is the
  same either way: intersect the two interval lists for each line. Each output run gets syntax
  foreground + font style, and diff background where an Inline Change (or Change Block line) covers
  it. Neither engine needs to know about diffs.
- **Prior art: delta** (the git pager) does exactly this on top of syntect. `superimpose_style_sections`
  takes "foreground styles computed by syntect" and "styles computed by delta reflecting within-line
  edits" and "outputs one stream with a single style assigned to each character". It keeps the
  syntect foreground and delta's background
  ([delta src/paint.rs](https://github.com/dandavison/delta/blob/main/src/paint.rs)).
- **syntect** already has helpers. `Style` is `{foreground, background, font_style}`
  ([source](https://docs.rs/syntect/5.3.0/syntect/highlighting/struct.Style.html)).
  [`util::modify_range(v, r, StyleModifier)`](https://docs.rs/syntect/5.3.0/syntect/util/fn.modify_range.html)
  restyles a byte range of an already-highlighted line, and `util::split_at` is the primitive behind it.
- **tree-sitter** gives
  [`HighlightEvent::Source{start,end}`](https://docs.rs/tree-sitter-highlight/0.27.0/tree_sitter_highlight/enum.HighlightEvent.html)
  byte ranges, with a stack of `HighlightStart`/`HighlightEnd` events. Flattening that into
  `(range, capture)` runs is a few lines. The Inline Change merge is then the same interval
  intersection.
- Design note for the renderer ticket: the syntax theme should only set the foreground. The Change
  Block / Inline Change backgrounds then stay legible under every syntax theme. syntect themes can
  carry backgrounds, which we would ignore.

### 4. Grammar bundling and binary size

All numbers are measured stripped release builds (`strip = true`, x86_64 Linux):

| Probe | Size |
|---|---|
| empty Rust bin | 0.35 MB |
| syntect defaults (75 syntaxes, onig) | 1.57 MB |
| two-face extra (220 syntaxes, onig) | 2.26 MB |
| tree-sitter runtime + highlight + Rust grammar | 3.52 MB |
| … + JavaScript grammar | 3.96 MB |
| arborium, Rust + JS | 4.88 MB |
| arborium, 30 common languages¹ | **48.5 MB** (5.0 MB gzip -9, 3.0 MB xz -9) |
| arborium, all 112 languages | **167 MB** |
| difftastic 0.71.0 release `difft` (65 langs), stripped | 117 MB (release tarball 11.7 MB) ([release](https://github.com/Wilfred/difftastic/releases/tag/0.71.0)) |

¹ rust, javascript, typescript, tsx, python, go, java, c, cpp, c-sharp, kotlin, swift, ruby, php, bash,
html, css, scss, json, yaml, toml, markdown, sql, xml, lua, scala, dockerfile, make, diff, ini.

- **The size is static data, not code.** In the 30-language binary, `.rodata` is 44.5 MB of 48.5 MB
  (`size -A`). Optimisation level and LTO will not shrink it. Grammar sizes vary widely: the generated
  `parser.c` is 34 MB for D, 32 MB for F#, 29 MB for C#, 25 MB for C++, and much less for JSON/TOML
  (e.g. [arborium-d parser.c](https://docs.rs/crate/arborium-d/2.18.2/source/grammar/src/parser.c),
  [arborium-cpp](https://crates.io/crates/arborium-cpp); measured with `du -b` on the crate sources). Choosing which grammars to ship is the only lever.
- The tables compress about 10×, so download size (dmg/AppImage/tarball) is modest. Installed size
  is not.
- Mitigations, in order of laziness:
  1. A curated list of grammars.
  2. Drop the heaviest grammars that are rarely needed.
  3. Later, Helix-style dynamic loading of grammar `.so`/`.dylib` files
     ([Helix docs](https://docs.helix-editor.com/guides/adding_languages.html)).
- syntect's per-syntax cost is tiny: two-face's docs put the extra syntaxes at about 0.6 MiB over
  syntect's defaults ([docs.rs/two-face](https://docs.rs/two-face/latest/two_face/)), and we measured
  +0.7 MB.

### 5. Would tree-sitter parse trees serve AST-aware diff later?

**Yes. This is the main strategic reason to choose tree-sitter.**

- difftastic converts tree-sitter's parse tree into its own uniform tree, where "everything is either
  an atom … or a list". It uses per-language `atom_nodes` and `delimiter_tokens` to flatten or regroup
  nodes ([difftastic: parsing](https://difftastic.wilfred.me.uk/parsing.html)). It then treats the diff
  as "a route finding problem on a directed acyclic graph" solved with Dijkstra
  ([difftastic: diffing](https://difftastic.wilfred.me.uk/diffing.html)).
- difftastic also runs each grammar's **highlight query** against the same tree. It builds
  `HighlightedNodeIds` (keyword / string / type / comment IDs) to colour its output, and caches compiled
  queries because "Constructing a tree sitter query is relatively expensive: it can take tens of
  milliseconds" ([tree_sitter_parser.rs](https://github.com/Wilfred/difftastic/blob/master/src/parse/tree_sitter_parser.rs)).
  This is the shape we want: **one parse per Side feeds both highlighting and structural diff.**
- The guards difftastic needs tell us what our future AST-diff ticket will need:
  - `byte-limit` (default 1,000,000): "Use a line-oriented diff if either input file exceeds this size".
  - `graph-limit` (default 3,000,000 vertices): "controls the worst case runtime and memory usage".
  - `parse-error-limit` (default 0): "any parse error will make difftastic use a line-oriented diff"
    ([options.rs](https://github.com/Wilfred/difftastic/blob/master/src/options.rs)).

  So AST-aware diff is a mode layered over a line diff, not a replacement for it. Highlighting has no
  such limits: tree-sitter highlights through errors (§2).
- syntect produces only a flat stream of scope push/pop operations per line, with no tree. It cannot
  feed structural diff.

### 6. Licensing

- `tree-sitter`, `tree-sitter-highlight`: MIT. Grammar crates such as `tree-sitter-rust` and
  `tree-sitter-javascript`: MIT ([crates.io](https://crates.io/crates/tree-sitter)). Each grammar
  carries its own license, so check every one we add.
- arborium: MIT OR Apache-2.0. Bundled grammars "retain their original licenses" and the permissive
  ones (MIT/Apache-2.0/CC0/Unlicense) are the documented set
  ([GitHub](https://github.com/bearcove/arborium)). **Gotcha:** the `all-languages` feature includes
  `lang-nginx`, which is GPL ([docs.rs/arborium](https://docs.rs/arborium/2.18.2/arborium/);
  [Cargo.toml](https://docs.rs/crate/arborium/2.18.2/source/Cargo.toml)). List features explicitly and
  never use `all-languages`.
- tree-house (Helix's highlighter) and syntastica are **MPL-2.0**
  ([tree-house](https://github.com/helix-editor/tree-house); [crates.io/syntastica](https://crates.io/crates/syntastica)).
  File-level copyleft is usable as a dependency, but it is a heavier licence than MIT. giallo is
  EUPL-1.2 ([crates.io/giallo](https://crates.io/crates/giallo)). Avoid it.
- syntect: MIT ([GitHub](https://github.com/trishume/syntect)). two-face: MIT OR Apache-2.0. Its
  embedded grammar/theme licenses are Sublime's custom permissive license, MIT, BSD-2/3, Apache-2.0,
  Unlicense and WTFPL. MIT/BSD/Apache "require acknowledgement"
  ([two-face acknowledgement::LicenseType](https://docs.rs/two-face/latest/two_face/acknowledgement/enum.LicenseType.html)).
  The `onig_sys` crate is MIT, and the Oniguruma C library it bundles is BSD-2-Clause
  ([crates.io/onig_sys](https://crates.io/crates/onig_sys); `oniguruma/COPYING` in the crate source).

### 7. Alternatives considered

| Option | Verdict |
|---|---|
| Official `tree-sitter` + `tree-sitter-highlight` + grammar crates | **Pick.** Stock MIT types, the same ones difftastic uses. We hand-pick grammars. |
| arborium | Good fallback if managing grammar crate versions gets painful: one dependency, lockstep-versioned grammars and queries, raw `Span` API. Costs: a forked tree-sitter runtime (`arborium-tree-sitter`), and its `highlight_spans` was 1.9× slower than `tree-sitter-highlight` on Rust in our run. |
| tree-house (Helix) | Robust incremental highlighter across injections, but MPL-2.0 and built around Helix's `.so` grammar loading. Read it for ideas when we write our own `QueryCursor` highlighter. |
| tree-sitter-language-pack | Downloads grammars at runtime. Wrong model for an offline desktop app. |
| syntect + two-face | Smallest binary (2.3 MB for 220 syntaxes) and theme-ready output. Loses on speed (3.7–8× slower with onig, 13–25× with fancy), cannot highlight only the viewport, and has no path to AST diff. |

## Risks

1. **Binary size.** Measured at about 1.5 MB per grammar on average for the 30-language set, with a
   heavy tail (C++, C#, F#, D, Haskell). Mitigate with a curated list, and dynamic loading if it ever
   matters.
2. **Grammar crate version skew and ABI.** The runtime accepts grammar ABI 13–15
   (`LANGUAGE_VERSION` = 15, `MIN_COMPATIBLE_LANGUAGE_VERSION` = 13 in tree-sitter 0.27.0,
   [docs.rs](https://docs.rs/tree-sitter/0.27.0/tree_sitter/constant.MIN_COMPATIBLE_LANGUAGE_VERSION.html)).
   Some grammar crates lag (tree-sitter-typescript last released 2024-11). Mitigations: pin versions,
   check `set_language` errors in a test that loads every shipped grammar, or switch to arborium's
   lockstep bundle.
3. **Query quality and capture-name drift.** Upstream `highlights.scm` files vary in coverage and
   naming. Helix and Zed maintain their own. Expect to vendor or patch queries for a few languages,
   and keep a small capture-name → theme-slot table.
4. **We must write our own highlighter to get viewport highlighting and tree sharing.**
   `tree-sitter-highlight` parses internally and works on the whole file. The workaround is a
   `QueryCursor` loop with `set_byte_range` over our own tree. Until then `tree-sitter-highlight` on
   both Sides is fast enough (≤0.63 s per 100k-line Side, off the UI thread).
5. **C toolchain in CI on Linux and macOS.** Every grammar compiles C via `cc`. syntect also needs C
   unless we accept fancy-regex's 3.5× slowdown, so this does not distinguish the two.
6. **Pathological inputs.** Minified or huge single-line files. tree-sitter offers parse/query
   progress callbacks for cancellation (`ParseOptions::progress_callback`,
   `QueryCursorOptions::progress_callback` in 0.27.0). Add a size cap that falls back to plain text,
   the same pattern as difftastic's `byte-limit`.
7. **Benchmark caveats.** One Linux desktop and synthetic concatenated corpora (see below). The ratios between engines are the useful part.

## Benchmark

- **Machine:** Linux desktop, rustc 1.98.1, `--release`.
- **Crates:** syntect 5.3.0, two-face 0.5.2+bat-0.26.1, tree-sitter / tree-sitter-highlight 0.27.0,
  tree-sitter-rust 0.24.2, tree-sitter-javascript 0.25.0, arborium 2.18.2.
- **Corpora:**
  - Rust: 99,838 lines (3.4 MB) made by concatenating `.rs` files from the local cargo registry.
  - JS: the first 100,000 lines (4.9 MB) of
    [typescript@5.6.3/lib/typescript.js](https://unpkg.com/typescript@5.6.3/lib/typescript.js).
- **Single-threaded, one Side at a time.** Theme: `base16-ocean.dark` for syntect. For tree-sitter, 26
  common capture names were configured; mapping them to colours costs nothing extra.
- **Span counts** (sanity check that work was done):
  - syntect: 694,553 (Rust) and 1,217,252 (JS) styled runs.
  - tree-sitter-highlight: 404,808 and 681,453 `HighlightStart` events.
  - arborium: 495,187 and 737,684 spans.
- The scratch project lived in `$TMPDIR/hlbench` and `$TMPDIR/arbsize` and is not in
  the repo. Core loops, for reproducing:

```rust
// syntect: parse + style, line by line (state carries across lines)
let mut h = HighlightLines::new(syn, theme);
for line in LinesWithEndings::from(src) { spans += h.highlight_line(line, &ss)?.len(); }

// tree-sitter: full parse, then incremental re-parse after inserting "\n" mid-file
let tree = parser.parse(src, None).unwrap();
let mut t2 = tree.clone(); t2.edit(&InputEdit { start_byte: mid, old_end_byte: mid, new_end_byte: mid + 1, .. });
parser.parse(&edited, Some(&t2));

// tree-sitter-highlight: parse + highlight whole document
let mut cfg = HighlightConfiguration::new(tree_sitter_rust::LANGUAGE.into(), "rust",
    tree_sitter_rust::HIGHLIGHTS_QUERY, tree_sitter_rust::INJECTIONS_QUERY, "")?;
cfg.configure(NAMES);
for ev in Highlighter::new().highlight(&cfg, src.as_bytes(), None, None, |_| None)? { /* count */ }

// arborium
arborium::Highlighter::new().highlight_spans("rust", &src)?;
```

Binary-size probes are minimal `main`s that each call one engine on `argv[1]`. They were built with
`strip = true`, and sizes were read with `stat` and `size -A`.
