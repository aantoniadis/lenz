# Security and performance practices for a Rust viewer of untrusted repositories

Researched 2026-10-08. Versions checked: git 2.55.0, gh 2.102.0, rustc/cargo 1.98.1 (local); crates as locked in `Cargo.lock`:
ratatui 0.30.2, ratatui-core 0.1.2, ratatui-widgets 0.3.2, crossterm 0.29.0, unicode-width 0.2.2, image 0.25.10, resvg/usvg 0.46.0,
roxmltree 0.21.1, pulldown-cmark 0.13.4, tree-sitter 0.27.0, tempfile 3.27.0, memchr 2.8.3, rustc-hash 1.1.0 and 2.1.3,
calloop 0.14.3 (git `3759371f`). Lenz paths are given as `crates/...:line` at the working tree on 2026-10-08 (the working tree was being edited
concurrently by another agent while this was written, so line numbers may have shifted). "Unverified" means I could not read a primary source or run a check in the research budget.

Scope: what Lenz should do because it shows untrusted content (repo files, commit messages, ref names, paths, `gh` JSON, theme TOML) and runs
`git` inside repositories it did not create. No application code was changed. Nothing here was benchmarked; every performance statement is a
source claim or a recommendation to measure with `docs/benchmarking.md`.

## Recommendation

Ordered by risk removed per unit of work. Items 1-4 close concrete paths in the current code; the rest are hardening.

1. **Stop executing textconv/external-diff commands that come from repo-local git config.** Lenz does exactly that today:
   `crates/app/src/changeset.rs:314` runs `git config -z --get-regexp '^diff\..+\.(textconv|binary)$'` (all scopes, including `.git/config`),
   and `changeset.rs:228-233,290` then runs the command through `sh -c` (working-tree side) or `git cat-file --textconv` (blob side). Only
   `safe.directory`, `safe.bareRepository` and `uploadpack.packObjectsHook` are documented as "protected configuration only" (git-config(1) 2.55.0;
   `grep -i protected` finds exactly these three); every other key relevant here is honored from the repo
   ([finding 2.3](#23-which-repo-local-keys-execute-code)). Accept a driver only when `git config --show-scope` reports `system`, `global` or
   `command`, show a visible "textconv from repo config not run" notice otherwise, and pass `--no-textconv --no-ext-diff` on every
   listing/patch call. This is the top finding because the ownership check behind CVE-2022-24765 does not help when the user owns the directory.
2. **Give every `git` spawn a fixed hardened prefix**, in the one place that builds the command (`crates/app/src/git.rs:15`):
   `--no-optional-locks` on every spawn and `GIT_TERMINAL_PROMPT=0` on the network fetch (as implemented after this note: an env override on every spawn
   makes std copy the whole parent environment, about 70 µs per process with a desktop environment, so the lock setting is passed as the equivalent argument), and `-c core.fsmonitor=false -c core.hooksPath=/dev/null` on read-only calls, because command-line `-c` overrides config files
   ([finding 2.2](#22-environment-and-flags-for-a-read-only-spawn)). Decide separately whether `GIT_NO_LAZY_FETCH=1` is wanted (it stops network
   fetches of missing objects in partial clones). Do not add this prefix to the merge/apply workflows that intentionally run the user's own hooks.
3. **Put `--end-of-options` before every revision, ref or path that originates in repo data** (ref names may begin with `-`). My grep for it
   in `crates/` found no hit in the lines I saw; audit all `Command::new("git")` argument lists ([finding 2.1](#21-argument-and-output-framing)).
4. **Gate `git cat-file --batch -Z` on git >= 2.42.** `-Z` first appears in the v2.42.0 docs and is absent from v2.41.0
   ([finding 2.4](#24-cat-file---batch--z-version)), but `crates/app/src/git.rs:10` allows git 2.30. Use `-Z` only after a version check, or keep the
   LF protocol with the existing injection fix.
5. **Sanitize displayed text once, in the shared core, replacing (not deleting) dangerous code points with a visible marker.** The lead's decision
   stands, but the ratatui premise needs correcting: ratatui-core 0.1.2 and ratatui-widgets 0.3.2 already drop bidi controls silently, by its zero-width filter, not by
   `is_control` ([finding 1.2](#12-what-ratatui-filters-verified-by-probe)). Silent removal hides a Trojan Source attack instead of surfacing it;
   the GUI renders bidi; and clipboard/CLI-error paths never touch a ratatui buffer. Also cap runs of combining marks per grapheme: nothing in the
   TUI bounds them ([finding 1.4](#14-combining-mark-floods)).
6. **Bound decoders explicitly.** Set `image::Limits` width, height and `max_alloc` on the top-level decode path; cap SVGZ output before
   `usvg::Tree::from_data` (it uses unbounded `read_to_end`); keep `resources_dir = None` and the `resolve_string => None` override you already have
   ([finding 5](#5-decoder-resource-limits)).
7. **Supply chain:** the `calloop` patch already gained `rev = "3759371fee14c40066d64777e2a36b6ffcc22590"` in the working tree (`Cargo.toml:19-20`, uncommitted at read time), so only commit it; build releases with `--locked`; run
   `cargo deny check` (advisories, bans, sources) and `cargo audit` in CI; defer `cargo vet` ([finding 7](#7-supply-chain)).
8. **Crash-freedom:** workspace lints with `unsafe_code = "deny"` (one `unsafe` found, `highlight.rs:377`), clippy `unwrap_used`/`expect_used` at
   `warn` with the documented test opt-outs, and `indexing_slicing`/`arithmetic_side_effects` only in parsers of untrusted bytes. Keep
   `overflow-checks` off in `dist` unless a benchmark shows the cost is negligible (I measured nothing). Handle `BrokenPipe` on stdout
   ([finding 6](#6-crash-freedom)).
9. **gh:** keep `GH_PROMPT_DISABLED` (`pr.rs:157`), add `GH_NO_UPDATE_NOTIFIER=1` and `GH_NO_EXTENSION_UPDATE_NOTIFIER=1`, and run all `gh` JSON strings
   through the same sanitizer ([finding 3](#3-gh-cli)).
10. **Atomic writes:** keep tempfile-in-target-directory + rename, add `sync_all` on the file and `fsync` on the directory for user documents,
    and copy the original mode before persisting ([finding 4](#4-atomic-file-writes-on-unix)).
11. **Performance:** none of the above should cost measurable time if the sanitizer has a clean-string fast path returning `Cow::Borrowed`
    ([finding 8](#8-performance)). Keep SipHash (std default) for maps keyed by repo-controlled strings. Re-measure per `AGENTS.md`.
12. **TUI:** the panic-hook recipe is already correct ([finding 9](#9-ratatuicrossterm)); remaining work is confirming resize handling and
    sanitizing bracketed-paste text.

## 1. Terminal injection

### 1.1 Why raw control characters are dangerous

- xterm treats `ESC ] 52 ; Pc ; Pd ST` (OSC 52, "Manipulate Selection Data") as: `Pd` is base64 and "becomes the new selection, which is then
  available for pasting by other applications"; if `Pd` is `?` "xterm replies to the host with the selection data" (the reply arrives on the
  application's input stream); any other value "clears the selection". The doc says these controls may be disabled with `allowWindowOps`
  ([xterm ctlseqs, OSC Ps ; Pt ST, Ps = 52](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)).
- `CSI Ps ; Ps ; Ps t` is "Window manipulation (XTWINOPS)" and "may be disabled using the allowWindowOps resource"; it includes report operations for the
  window title and icon label (Ps = 20 and 21; "Functions using CSI"). A report is written into the input stream, so text printed to a terminal can cause
  bytes to appear as if typed. DECRQSS (`DCS $ q Pt ST`, "Device-Control functions") is likewise answered by a `DCS` reply.
- The document lists the 8-bit C1 controls (0x80-0x9F), where `CSI` is 0x9B ("C1 (8-Bit) Control Characters"). Whether a given terminal in UTF-8
  mode honors U+009B is terminal-specific: unverified. Strip U+0080-U+009F regardless.
- I did not find an OSC 8 (hyperlink) entry in the copy of ctlseqs I read (text search for "hyperlink" returned nothing). Document date unverified.

### 1.2 What ratatui filters (verified by probe)

- `Buffer::set_stringn` segments into graphemes, drops any grapheme containing `char::is_control`, drops any grapheme with `cell_width() == 0`,
  and truncates to the width ([`ratatui-core-0.1.2/src/buffer/buffer.rs:333-351`](#probe)). `Span::styled_graphemes` applies only the `is_control` filter
  (`ratatui-core-0.1.2/src/text/span.rs:314`). `Paragraph` rendering then skips `width == 0` graphemes and calls `Cell::set_symbol` for the rest
  (`ratatui-widgets-0.3.2/src/paragraph.rs:463-471`).
- `char::is_control` is Unicode category Cc: C0 (U+0000-U+001F), DEL, and C1 (U+0080-U+009F). Bidi controls (U+202A-U+202E, U+2066-U+2069),
  U+200B-U+200F and U+FEFF are category Cf, **not** Cc, so `is_control` does not remove them.
- Probe result (ratatui-core 0.1.2 with unicode-width 0.2.2, `Buffer::set_string` then reading cells; commands in [Probe](#probe)):

  | Input | Cells after `set_string` |
  |---|---|
  | `a U+202E b` | `ab` (dropped) |
  | `a U+2066 b U+2069 c` | `abc` (dropped) |
  | `a U+200B b`, `a U+200E b U+FEFF c` | `ab`, `abc` (dropped) |
  | `a ESC [31m b` | `a[31mb` (ESC dropped; rest is inert text) |
  | `a U+009B 31m b`, `a BEL b` | `a31mb`, `ab` |
  | `e U+0301` | kept (one cell holds two chars) |

  So in the TUI, escape sequences are neutralized and bidi controls vanish through the width filter. Only U+202E, U+2066, U+2069 were probed; the
  other bidi controls are the same category and expected to behave alike (unverified). A widget that writes `Cell::set_symbol` without the width
  check, or any output outside the buffer (OSC 52 payload, stderr messages, logs, clipboard), is not covered.
- The GUI path: cosmic-text 0.19 applies `unicode-bidi` (stated by the lead; I did not re-read cosmic-text), so bidi controls there do reorder
  rendered text.

### 1.3 Bidi and Trojan Source

- CVE-2021-42574: "The attack is to use control characters embedded in comments and strings to reorder source code characters in a way that
  changes its logic." CVE-2021-42694 covers homoglyph identifiers. Paper: Boucher and Anderson, "Trojan Source: Invisible Vulnerabilities",
  USENIX Security 2023 ([trojansource.codes](https://trojansource.codes/)).
- The authors' recommendation for editors and repository frontends: "make bidirectional control characters and mixed-script confusable characters
  perceptible with visual symbols or warnings" (same page). That favors a visible marker (for example `<U+202E>` or U+FFFD styled as an error) over
  silent deletion, which is what ratatui does.
- Homoglyphs (CVE-2021-42694) are not handled by any filter above; detecting them needs mixed-script analysis (out of scope; unverified how
  much a viewer should do).

### 1.4 Combining-mark floods

- UAX #15 section 13 (Stream-Safe Text Format, Unicode 18.0.0 rev 58 as fetched) defines safe text as having no more than 30 non-starters in a
  row after NFKD, so that implementations can process text with small fixed buffers. The document says "Such extremely long sequences of combining marks are not illegal,
  even though for all practical purposes they are not meaningful", and that 30 is "significantly beyond what is required for any linguistic or technical
  usage". The Stream-Safe process inserts U+034F (CGJ) after 30 non-starters
  ([UAX #15, Stream-Safe Text Format](https://www.unicode.org/reports/tr15/#Stream_Safe_Text_Format)).
- ratatui does not apply it. Probe: `e` followed by 100,000 U+0301 produced one cell whose symbol held 100,001 chars, in about 12 ms; that whole
  string is then re-emitted to the terminal whenever the cell changes. Cap marks per base grapheme in the sanitizer (30 is the documented
  upper bound; a much smaller cap such as 8 is a product choice, not a sourced figure). GUI shaping cost for huge clusters: unverified.
- Zero-width characters are dropped by ratatui (probe above), so floods of them cost nothing in the TUI, but they let two visually identical
  lines differ. Surface them as markers in diff views if exact-content review matters.

### 1.5 Where untrusted strings still reach a terminal unfiltered

By inference, not audited: CLI error messages interpolate paths, ref names and commands (for example the textconv error in `changeset.rs` `textconv_file`, and `cli.rs` errors) and
go to stderr, which is a terminal. Pass them through the same sanitizer. OSC 52 output (TUI clipboard) must only ever send the write form with
base64 of the user's selection, never `?`. Bracketed-paste text arriving as `Event::Paste` (`crates/tui/src/chrome.rs:153`) is user input but
should be sanitized before being stored or displayed.

## 2. Git hardening for a read-only viewer in untrusted repositories

All git facts below are from the local git 2.55.0 man pages unless a tagged URL is given; the online equivalents are
[git(1)](https://git-scm.com/docs/git), [git-config(1)](https://git-scm.com/docs/git-config), [gitcli(7)](https://git-scm.com/docs/gitcli),
[git-cat-file(1)](https://git-scm.com/docs/git-cat-file), [githooks(5)](https://git-scm.com/docs/githooks).

### 2.1 Argument and output framing

- `--end-of-options`: "Because `--` disambiguates revisions and paths in some commands, it cannot be used for those commands to separate
  options and revisions. You can use `--end-of-options` for this (it also works for commands that do not distinguish between revisions in
  paths, in which case it is simply an alias for `--`)"; "When writing a script that is expected to handle random user-input, it is a good
  practice to make it explicit which arguments are which" (gitcli(7), "Disambiguating..." list).
- `-z` (NUL-terminated records) removes quoting ambiguity for paths in `diff --raw`, `status --porcelain=v2`, `ls-files`, `diff-tree`
  (already used: `changeset.rs`, `conflicts.rs`, `blame.rs`).

### 2.2 Environment and flags for a read-only spawn

| Setting | What the docs say | Use |
|---|---|---|
| `GIT_TERMINAL_PROMPT=0` | "If this Boolean environment variable is set to false, git will not prompt on the terminal" (git(1)) | network calls (any `Command::env` copies the parent environment per spawn) |
| `GIT_OPTIONAL_LOCKS=0` | "Git will complete any requested operation without performing any optional sub-operations that require taking a lock ... will prevent `git status` from refreshing the index as a side effect" (git(1)); `--no-optional-locks` is equivalent | always |
| `-c name=value` | "will override values from configuration files" (git(1) `-c`) | `-c core.fsmonitor=false -c core.hooksPath=/dev/null` |
| `--no-ext-diff`, `--no-textconv` | "Disallow external diff drivers"; textconv "enabled by default only for git-diff(1) and git-log(1), but not for ... diff plumbing commands" (git-diff(1)) | all diff calls |
| `GIT_NO_LAZY_FETCH=1` | "tells Git not to lazily fetch missing objects from the promisor remote on demand" (git(1)) | product decision |
| `GIT_NO_REPLACE_OBJECTS` | "ignore replacement refs" (git(1)) | optional; `refs/replace/*` can make displayed content differ from the real object (inference) |
| `GIT_PAGER` | empty or `cat` disables the pager (git(1)) | moot when stdout is a pipe (that pager is only for terminals: unverified) |

`GIT_OPTIONAL_LOCKS=0` does not obviously suppress a `core.fsmonitor` hook, which runs while refreshing the in-memory index; treat it as
independent of the `-c core.fsmonitor=false` override (unverified).

### 2.3 Which repo-local keys execute code

Each entry is the git-config(1) text for the key; "repo-local" means it is honored from `.git/config` (git-config(1) 2.55.0 restricts only
`safe.directory`, `safe.bareRepository` and `uploadpack.packObjectsHook` to protected scopes; `grep -i protected` finds no other key).

| Key | Executes | Runs under a read-only viewer? |
|---|---|---|
| `core.fsmonitor` | "Otherwise, this variable contains the pathname of the 'fsmonitor' hook command" (builtin daemon only on Windows and macOS) | yes, when git refreshes the index (`status`, `diff`) |
| `diff.<driver>.textconv` | "The command that the diff driver should call to generate the text-converted version of a file" | yes for `git diff`/`log` patches and `cat-file --textconv`; **Lenz also runs it itself** |
| `diff.<driver>.command`, `diff.external` | custom/external diff program | patch output only; blocked by `--no-ext-diff` |
| `filter.<driver>.smudge` | "convert the content of a blob object to a worktree file upon checkout" | only with `cat-file --filters` or checkout |
| `core.hooksPath`, `.git/hooks/*` | hooks | only commands that fire hooks; read-only listing does not, but `post-index-change` ("invoked when the index is written") and `reference-transaction` (any ref update) show why writes matter ([githooks(5)](https://git-scm.com/docs/githooks)) |
| `core.pager` | "meant to be interpreted by the shell" | only when git pages |
| `core.sshCommand` | replaces `ssh` for fetch/push | network operations only |
| `credential.helper` | "may also be ... shell commands" if prefixed `!` | network/auth only |
| `include.path`, `includeIf.<cond>.path` | pulls in another config file, so any key above | indirectly |

`.gitattributes` inside the repository selects a driver by name (`diff=<driver>`), but the driver's command is defined by config. Inference
from git-config(1) and gitattributes(5), not re-read: a cloned repository cannot define a textconv command on its own, because `git clone` does
not copy `.git/config`. The risk applies to directories obtained as archives, copies or shared volumes that include `.git/config`.

### 2.4 `cat-file --batch -Z` version

- `-Z` ("input and output is NUL-delimited") is present in [v2.42.0](https://github.com/git/git/blob/v2.42.0/Documentation/git-cat-file.txt)
  and absent from [v2.41.0](https://github.com/git/git/blob/v2.41.0/Documentation/git-cat-file.txt) (checked v2.39, v2.40, v2.41 absent; v2.42 present).
- Its predecessor `-z` (input only, now documented as deprecated "as the output can otherwise be ambiguous") is absent from
  [v2.37.0](https://github.com/git/git/blob/v2.37.0/Documentation/git-cat-file.txt) and present in
  [v2.38.0, L216-219](https://github.com/git/git/blob/v2.38.0/Documentation/git-cat-file.txt#L216-L219).
- Lenz's minimum is git 2.30 (`crates/app/src/git.rs:10`), so `-Z` needs a runtime gate (`git::version()` exists, `git.rs:30`).

### 2.5 Why a viewer cannot fully defend a hostile `.git/config`

- CVE-2022-24765 (fixed in 2.30.3 ... 2.35.2): "On multi-user machines, Git users might find themselves unexpectedly in a Git worktree ...
  Merely having a Git-aware prompt that runs `git status` (or `git diff`) ... or opening such a directory in an editor or IDE ... will potentially
  run commands defined by that other user" ([v2.30.3 notes](https://github.com/git/git/blob/v2.30.3/Documentation/RelNotes/2.30.3.txt);
  [v2.35.2 notes](https://github.com/git/git/blob/v2.35.2/Documentation/RelNotes/2.35.2.txt)). This is the same class as a viewer.
- The fix is an ownership check: "By default, Git will refuse to even parse a Git config of a repository owned by someone else, let alone run
  its hooks"; `safe.directory` exceptions are "only respected in protected configuration" (system, global, command scopes) so a repository
  cannot grant itself trust (git-config(1) `safe.directory`, SCOPES).
- It does not protect a repository the user owns: after the user copies or extracts a hostile tree, git will run its config. A viewer can reduce
  exposure (command-scope `-c` overrides, scope filtering above) but cannot enumerate every key that may execute code across git versions, so the
  viewer's own `sh -c` for textconv is the avoidable part. `git config --show-scope` (listed in the 2.55.0 git-config(1) man page) prints the scope of each value.

### 2.6 Minimal recommended set

`GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0`, `-c core.fsmonitor=false`, `-c core.hooksPath=/dev/null`, `--no-ext-diff --no-textconv`,
`--end-of-options` before untrusted operands, `-z` framing, `-Z` only on git >= 2.42, and scope-filtered textconv. `core.pager`,
`credential.helper`, `core.sshCommand`, and `filter.*.smudge` need no mitigation for listing/reading if network and `--filters` are not used.

## 3. gh CLI

`gh help environment` (gh 2.102.0) documents: `GH_PROMPT_DISABLED` ("set to any value to disable interactive prompting in the terminal"),
`GH_NO_UPDATE_NOTIFIER` ("disable GitHub CLI update notifications ... checks for new versions once every 24 hours ... notice is displayed on
standard error"), `GH_NO_EXTENSION_UPDATE_NOTIFIER`, `GH_REPO`/`GH_HOST` (select repository and host), `GH_PAGER`, `NO_COLOR`, `GH_DEBUG` (`api`
logs HTTP traffic to stderr), `GH_FORCE_TTY` (forces terminal-style output; do not set). Lenz sets `GH_PROMPT_DISABLED` at `crates/app/src/pr.rs:153-157`.

- JSON trust: `gh ... --json` output contains strings authored by other GitHub users (titles, bodies, logins, branch names, labels). No gh
  document states that these are sanitized (I found none); treat every string as untrusted and run it through the sanitizer.
- `GH_REPO` is documented for "commands that otherwise operate on a local repository", so `gh` run with the cwd in the untrusted repository
  consults it (and probably invokes `git`: inference), which makes the git environment of finding 2.2 relevant to `gh` children too; env is inherited.
- Whether `gh` pages when stdout is a pipe is not stated in `gh help environment`: unverified.

## 4. Atomic file writes on Unix

- `O_CREAT|O_EXCL`: "if path is a symbolic link, then open() fails regardless of where the symbolic link points"; it fails with `EEXIST` if the
  path exists; the mode is `mode & ~umask` ([open(2)](https://man7.org/linux/man-pages/man2/open.2.html), `O_EXCL`, `mode`). tempfile 3.27.0
  uses `create_new(true)` and mode `0o600` by default (`tempfile-3.27.0/src/file/imp/unix.rs:18,23`; `src/lib.rs:407-409`).
- A plain `fs::write(path)` is `O_CREAT|O_TRUNC` without `O_EXCL`, so it follows symlinks (standard `open` semantics; the O_EXCL text above is the
  contrast). `crates/app/src/pr.rs:659` writes a fixed temp path with `fs::write` then renames; low risk in a user-owned config dir, but
  `create_new` would remove the question.
- `rename` replacement: tempfile documents "If a file exists at the target path, persist will atomically replace it"
  (`tempfile-3.27.0/src/file/mod.rs:170,734`) and that `persist_noclobber` "is not guaranteed to be atomic on all platforms" (`mod.rs:788-789`).
  Rename must stay on one filesystem, so create the temp in the target's directory (`Builder::tempfile_in`; EXDEV behavior unverified, rename(2) not fetched).
- Durability: "Calling `fsync()` does not necessarily ensure that the entry in the directory containing the file has also reached disk. For that an
  explicit `fsync()` on a file descriptor for the directory is also needed" ([fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html)). I found no
  `sync_all` in tempfile's `src/lib.rs`, `src/file/mod.rs` or README, so `persist` alone is atomic but not durable. Sequence: write, `sync_all`, `persist`,
  open parent dir, `sync_all`. Lenz's crash reports already do `sync_all` then `rename` (`diagnostics.rs:377-378`); no directory fsync seen.
- Mode: a new tempfile is `0600` (above); persisting over an existing `0644` file therefore changes its mode unless copied first (inference from the two
  facts; `set_permissions` before persist).
- TOCTOU limits: tempfile warns that a check before use "could be introduced" as a time-of-check to time-of-use bug unless atomic
  (`tempfile-3.27.0/src/lib.rs:652-653`). `rename` is atomic only for the final component; a symlink swapped into a parent directory between your
  check and the rename is not prevented by std. Stronger options (`openat2` with `RESOLVE_NO_SYMLINKS`, `cap-std`) were not researched: unverified.

## 5. Decoder resource limits

- `image` 0.25.10 `Limits { max_image_width, max_image_height, max_alloc }`; `Default` is no width/height limit and `max_alloc = Some(512 MiB)`
  (`image-0.25.10/src/io/limits.rs:33-56`). `max_alloc` "is non-strict by default and some decoders may ignore it" and the output buffer counts
  toward it (`limits.rs:38-42`); width/height "limit is strict". `ImageReader` starts with `Limits::default()` and offers `limits()`/`no_limits()`
  (`image_reader_type.rs:93,105,132-137`); `ImageDecoder::set_limits` exists (`io/decoder.rs:115`). `Limits::check_support` is a no-op in this
  version (`limits.rs:71-73`). Lenz applies `Limits::default().reserve_buffer` for the RGBA expansion (`crates/app/src/images.rs:181`) and
  `PngDecoder::with_limits` for SVG-embedded rasters; set explicit dimension limits on the main path too.
- usvg 0.46.0: error `ElementsLimitReached` — "We do not allow SVG with more than 1_000_000 elements for security reasons"
  (`usvg-0.46.0/src/parser/mod.rs:35-37`); `<use>` recursion stops at depth > 1024 (`parser/svgtree/parse.rs:182`).
  `Tree::from_data` sniffs gzip (`0x1f 0x8b`) and `decompress_svgz` uses `read_to_end` with no size cap (`parser/mod.rs:99-101,166-172`): a
  decompression bomb; bound the compressed input and the output yourself. roxmltree 0.21.1: DTD off by default ("for security reasons"),
  entity expansion depth 10, "still has checks for billion laughs", and `nodes_limit` defaults to `u32::MAX` (`roxmltree-0.21.1/src/parse.rs:69,330-346`);
  whether usvg passes a `nodes_limit` was not checked.
- usvg `Options::resources_dir` defaults to `None` (`options.rs:20`) but the default href resolver reads any existing path via `std::fs::read`
  (`parser/image.rs:85-95`), i.e. an SVG could embed local files; Lenz replaces `resolve_string` with `None` (`images.rs:134-138`). Keep that.
  Output size: Lenz clamps with `MAX_SIDE` before `Pixmap::new` (`images.rs:117-121`).
- pulldown-cmark 0.13.4 is "Safe; written in pure Rust with no unsafe blocks (except in the opt-in SIMD feature)" (`README.md:17`). The only depth
  limit I found is `LINK_MAX_NESTED_PARENS = 32` in link destinations, added against denial of service (`src/parse.rs:41-46`). Block-quote/list nesting
  limits were not found: unverified. The event stream is flat `Start/End`, so recursion risk sits in Lenz's consumer; `markdown::Limits`
  (`crates/app/src/markdown.rs:41-55`: 4 MiB, 100,000 events, 200,000 segments per side) is the actual guard.
- tree-sitter 0.27.0: `ParseOptions::progress_callback(FnMut(&ParseState) -> ControlFlow<()>)` cancels a parse
  (`tree-sitter-0.27.0/binding_rust/lib.rs:216-232`), `QueryCursor` has the same shape (`lib.rs:252-266`). My grep found no `set_timeout_micros` or
  cancellation-flag API in this binding version. Lenz uses the callback (`crates/app/src/highlight.rs:384-387`).

## 6. Crash-freedom

- `panic = "abort"` (dist profile, `Cargo.toml` `[profile.dist]` (lines 27-34 at read time)): a panic ends the process after the panic hook runs; destructors do not run, so `Drop` guards
  such as the TUI `Restore` (`crates/tui/src/main.rs`, `struct Restore`, about lines 108-116) never fire on a panic (standard semantics; the Cargo profile docs were not fetched).
  Consequences: every panic anywhere (workers included) is a crash, so untrusted-input panics are user-visible crashes; `catch_unwind` cannot
  contain them; and `release` (unwind) and `dist` (abort) behave differently, so run at least the parser fuzz/regression tests under `dist`.
- Clippy: `unwrap_used`, `expect_used`, `indexing_slicing`, `arithmetic_side_effects` are restriction lints (opt-in). The clippy configuration page
  confirms the test opt-outs `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-indexing-slicing-in-tests` and
  `arithmetic-side-effects-allowed` ([lint_configuration.md L94, L104, L207, L334](https://github.com/rust-lang/rust-clippy/blob/master/book/src/lint_configuration.md)).
  Lint descriptions were not fetched. My grep counted 1,298 lines with `unwrap()`/`expect(` in `crates/*/src/*.rs` (upper bound: it includes in-file test
  modules), so enable `unwrap_used`/`expect_used` at `warn` workspace-wide and reserve `indexing_slicing`/`arithmetic_side_effects` for modules that parse
  untrusted bytes (git output, framing, decoders, diff inputs). `[workspace.lints]` plus `[lints] workspace = true` per member and
  `#![forbid(unsafe_code)]`: syntax and semantics not fetched (unverified); note `forbid` cannot be locally re-allowed, and `highlight.rs:377` has
  the only `unsafe` my grep found (not inspected), so use `deny` for `lenz` and `forbid` for the engine and TUI crates.
- `overflow-checks`: verified by compiling the same program with rustc 1.98.1: `-O` wraps (`255u8 + 1` printed `0`), `-O -C overflow-checks=on` panics
  at runtime. The Perf Book's build-configuration page mentions overflow checks being off in release ([L53](https://github.com/nnethercote/perf-book/blob/master/src/build-configuration.md?plain=1#L53)).
  Runtime cost for Lenz: unmeasured. Checked arithmetic (`checked_*`, `saturating_*`) in the specific parsers is cheaper to justify than a global flag.
- Rust 2024 (compiled with `--edition 2024`, rustc 1.98.1; 2021 shown for contrast): `#[no_mangle]` errors "unsafe attribute used without unsafe";
  `std::env::set_var` errors E0133 "call to unsafe function `set_var` is unsafe and requires unsafe block"; a reference to `static mut` errors E0133 in both
  editions. Edition Guide pages were not fetched. Practical rule: never call `set_var`/`remove_var` after threads start; pass env through `Command::env`.
- SIGPIPE/EPIPE: Rust's runtime ignores SIGPIPE (std's startup `SIG_IGN`: not re-read; the probe shows the consequence), so `println!` on a closed pipe panics. Probe: an infinite `println!` loop piped to `head -1` exits 101 with
  "failed printing to stdout: Broken pipe (os error 32)" (`library/std/src/io/stdio.rs:1166`). Under `panic = "abort"` that panic becomes an abort
  (inference, not run) and Lenz's panic hook would record a crash for `lenz --help | head`. Use `writeln!` and treat `ErrorKind::BrokenPipe` as success on stdout.

## 7. Supply chain

- Git dependency forms: with only the URL, "Cargo assumes that we intend to use the latest commit on the default branch"; `branch`, `tag`, `rev` select
  more precisely; "Cargo locks the commits of `git` dependencies in `Cargo.lock` file at the time of their addition and checks for updates only when
  you run `cargo update` command" ([Cargo: specifying dependencies from git](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-dependencies-from-git-repositories)).
- Tags are not immutable: git-tag(1) "On Re-tagging" says if you never pushed "just re-tag it. Use -f to replace the old one" and that once published,
  others "will have already seen the old tag" (local man page 2.55.0, lines 222-231). Because `Cargo.lock` pins the commit, a moved tag only matters on
  `cargo update` or a fresh lock; `rev = "<sha>"` makes the pin visible in `Cargo.toml` and survives lock regeneration. The Cargo docs fetched do
  not state that tags can move.
- `[patch]` is read only from the workspace root: "Cargo only looks at the patch settings in the `Cargo.toml` manifest at the root of the workspace.
  Patch settings defined in dependencies will be ignored" ([Cargo: overriding dependencies, The \[patch\] section](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section)).
  That page does not say what a branchless git patch does on update. Evidence from this repo at HEAD: the patch was
  `calloop = { git = "https://github.com/zed-industries/calloop" }` and `Cargo.lock` records `source = "git+https://github.com/zed-industries/calloop#3759371f..."`
  with no `?rev=`/`?branch=`. By the general git-dependency rule above (inference for `[patch]`), `cargo update` would move it to the new tip of the remote default
  branch. The working tree now adds `rev = "3759371f..."` (`Cargo.toml:19-20`), the locked commit, which removes that drift.
  `async-task` is already pinned with `rev` (`Cargo.toml:18`).
- `--locked`: `cargo build --help` (cargo 1.98.1) says "Assert that `Cargo.lock` will remain unchanged"; use it for CI and `dist` builds so a stale lock fails
  instead of silently re-resolving.
- Tools: cargo-deny's README lists license and bans checks and a quickstart `cargo install --locked cargo-deny && cargo deny init && cargo deny check`
  ([README](https://github.com/EmbarkStudios/cargo-deny/blob/main/README.md); the advisories and sources check details were not read, see the
  [book](https://embarkstudios.github.io/cargo-deny/)). cargo-audit/RustSec ([rustsec.org](https://rustsec.org/)) and cargo-vet
  ([book](https://mozilla.github.io/cargo-vet/)) were not fetched: unverified. None of `cargo-audit`, `cargo-deny`, `cargo-vet` is installed here, so no
  audit was run. `Cargo.lock` carries duplicate versions (for example rustc-hash 1.1.0 and 2.1.3), which a bans check can report.

## 8. Performance

- Release profile levers, from the Perf Book build-configuration page ([source](https://github.com/nnethercote/perf-book/blob/master/src/build-configuration.md)):
  `codegen-units = 1` ([L86-92](https://github.com/nnethercote/perf-book/blob/master/src/build-configuration.md?plain=1#L86)), LTO "can improve runtime speed
  by 10-20% or more, and also reduce binary size" ([L103-130](https://github.com/nnethercote/perf-book/blob/master/src/build-configuration.md?plain=1#L103);
  thin vs fat described at L118-124), alternative allocators jemalloc/mimalloc "likely to increase binary size and compile times"
  ([L133-182](https://github.com/nnethercote/perf-book/blob/master/src/build-configuration.md?plain=1#L133)), PGO ([L227-238](https://github.com/nnethercote/perf-book/blob/master/src/build-configuration.md?plain=1#L227)).
  Lenz `dist` already has `lto = "thin"`, `codegen-units = 1`, `panic = "abort"` (`Cargo.toml` `[profile.dist]` (lines 27-34 at read time)). That page has no `panic` text, so the speed effect of
  `panic = "abort"` is unsourced here. Per `AGENTS.md`, treat fat LTO, a different allocator or PGO as experiments with baselines, not defaults.
- Allocation: `Rc`/`Arc` clone "does not involve an allocation" ([heap-allocations.md L69-83](https://github.com/nnethercote/perf-book/blob/master/src/heap-allocations.md?plain=1#L69)),
  `clone` ([L232](https://github.com/nnethercote/perf-book/blob/master/src/heap-allocations.md?plain=1#L232)) and `to_owned` ([L267](https://github.com/nnethercote/perf-book/blob/master/src/heap-allocations.md?plain=1#L267)) sections,
  `Cow` for "a mixture of borrowed and owned data" ([L282](https://github.com/nnethercote/perf-book/blob/master/src/heap-allocations.md?plain=1#L282)), and reusing collections
  ([L340](https://github.com/nnethercote/perf-book/blob/master/src/heap-allocations.md?plain=1#L340)). `Arc<str>` is not discussed there: unverified. The sanitizer should return
  `Cow::Borrowed` when the input has nothing to replace (the common case), so the hot path allocates nothing.
- Hashing: rustc-hash 2.1.3 "A speedy, non-cryptographic hashing algorithm"; "The hash map in `std` uses SipHash by default, which provides resistance
  against DOS attacks" (`rustc-hash-2.1.3/README.md:6-7`); in rustc "we're not really worried about DOS attempts" (`src/lib.rs:57-58`). Lenz uses neither
  rustc-hash nor memchr directly (grep: no hits). HashDoS needs attacker-chosen keys in a long-lived map; ref names, paths and author strings qualify,
  commit ids need grinding (inference). Use FxHash only for maps keyed by locally generated integers.
- Line splitting: memchr 2.8.3 uses AVX2 "when `std` is enabled ... if the CPU is determined to support it at runtime", SIMD on aarch64/wasm32, otherwise a fallback
  (`memchr-2.8.3/README.md:39-45`). Whether `str::lines` already uses it is unverified; benchmark `memchr_iter(b'\n')` against the current splitter before changing.
- Bounded caches, cancellation and viewport-limited work are covered by `docs/optimization-notebook.md`; nothing here contradicts them. Measurement tools
  (profilers, `dhat`, `cargo-pgo`) were not researched beyond the Perf Book pointers above.

## 9. ratatui/crossterm

- Panic hook: `ratatui::try_init()` calls `set_panic_hook()` first (`ratatui-0.30.2/src/init.rs:397-403`); the hook is `restore(); previous(info)` (`init.rs:566-571`).
  `restore()` only "disables raw mode" and "leaves the alternate screen" (`init.rs:505-510`), so mouse capture, bracketed paste and focus reporting must be undone by
  the app. Lenz does: `crates/tui/src/main.rs:119-128` installs a hook calling `restore_input_modes()` then the previous hook, after `try_init` (line 124), so on a panic
  the order is input modes, then ratatui restore, then the diagnostics hook, then the default message. This works under `panic = "abort"` because hooks run before the abort.
- Resize: crossterm delivers `Event::Resize`; I found no explicit handling in `crates/tui/src` (grep), so confirm drawing re-queries the frame area and test tiny sizes
  as `AGENTS.md` requires (unverified).
- Bracketed paste: `EnableBracketedPaste` and `Event::Paste(String)` exist in crossterm 0.29.0 (`crossterm-0.29.0/src/event.rs:421,562`); Lenz enables it
  (`main.rs:31`) and handles `Event::Paste` (`chrome.rs:153`). Sanitize pasted text; embedded paste-end markers as an injection vector: unverified.
- Keyboard enhancement: `KeyboardEnhancementFlags` are `DISAMBIGUATE_ESCAPE_CODES`, `REPORT_EVENT_TYPES`, `REPORT_ALTERNATE_KEYS`,
  `REPORT_ALL_KEYS_AS_ESCAPE_CODES` (`event.rs`, flag docs); `REPORT_ASSOCIATED_TEXT` "is not yet supported by crossterm". `supports_keyboard_enhancement()` "will block and
  possibly time out while `event::read`/`event::poll` are being called" on Unix (`crossterm-0.29.0/src/terminal/sys/unix.rs:183-188`): query it before starting the event loop.
  Lenz pushes no enhancement flags (grep), and filters `KeyEventKind::Release`; if flags are ever pushed, pop them in the same panic hook.

## Probe

Reproduction of the verified-by-experiment claims (scratch files only; nothing in the repository was touched):

- ratatui table and flood: a crate with `ratatui-core = "=0.1.2"` calling `Buffer::set_string` on the inputs in finding 1.2 and on `"e" + "\u{301}" * 100_000`,
  run with `cargo run --offline`; resolved `unicode-width 0.2.2`.
- Edition 2024: `rustc --edition 2024 --crate-type lib a.rs` with `std::env::set_var`, `#[no_mangle] pub extern "C" fn`, and a `static mut` reference; same file with `--edition 2021`.
- SIGPIPE: `fn main(){ loop { println!("line"); } }` built with `rustc -O`, run as `./b | head -1`; exit code 101.
- Overflow: `args().count() as u8 + 255` built with `rustc -O` and with `-O -C overflow-checks=on`.
- Tagged git docs: `curl` of `Documentation/git-cat-file.txt` at v2.37.0, v2.38.0, v2.39.0, v2.40.0, v2.41.0, v2.42.0.

## Not covered or unverified

cargo-audit, cargo-vet and the cargo-deny book; `[workspace.lints]` and `forbid(unsafe_code)` documentation; the Rust 2024 Edition Guide text;
rename(2)/openat2 semantics; clippy lint text; cosmic-text 0.19 bidi behavior (taken from the lead); `overflow-checks` and `panic = "abort"` runtime cost;
pulldown-cmark block nesting; terminal-specific handling of C1 and OSC 52 limits; native macOS and Windows behavior of every item.
