# Semantic provider compatibility

What each built-in provider is tested to do, populated from implementation tests only. A row without a tested
version has not been exercised against the real server; the reader still runs an installed one, with an
"untested version" notice. The reader never installs or upgrades a server.

Shared behaviour, for every provider:

- A provider without a readiness status (all but rust-analyzer) has its answers marked "may still be loading or
  indexing" for 10 s after it initializes, and a definition jumped to in that window says it may stop short.
- A selected name is asked about at its start.
- A provider that relies on the client for files it does not hold open (Ruby LSP; the others watch files
  themselves and are neither offered nor sent this) hears of files the reader's watcher sees change under its root
  (`workspace/didChangeWatchedFiles`: changed when the file exists, else deleted; a rename as both ends).
  Directory events and watcher overflow also reconcile the browsing root's inventory, reporting affected
  surviving files, additions and deletions. The shared declaration index and open symbol searches refresh too.
  Changed open Ruby documents also request a document-symbol refresh, which makes Ruby LSP update their index;
  closing a changed document reports its disk state again. These internal replies do not replace navigation results.
  Watch registration checks for missed changes without replaying every unchanged file. Folders outside watcher
  coverage remain a limit. Source-only branch checkouts keep the server running; after changing the bundle or
  Ruby runtime, use **Restart Language Server** to reload that environment.
- Dependency files show their origin, outside the project or inside it (`node_modules`, a project virtualenv).
- A dependency file outside the browsing root and every running server's workspace (a Cargo registry or git
  checkout, the standard library, installed packages: by the provider's origins) is served by a running server of
  its provider (the one last asked, whose answer opened it, first; then one whose workspace holds the browsing
  root), never by a server of its own for the dependency's workspace, which would load that whole workspace again
  (a second rust-analyzer for a GPUI-sized checkout is gigabytes). Opened with no server running (from recent
  places, say), it gets its own, as any project.
- Go to Implementation, Go to Type Definition, Call Hierarchy, Quick Documentation (hover) and occurrence
  highlighting are offered when the server's `initialize` capabilities say so; otherwise the command is disabled
  with that reason. Capabilities as the real servers reported them (2026-10-14, the `real_*` tests below):

  | Provider | Implementation | Type definition | Call hierarchy | Documentation | Occurrences |
  | --- | --- | --- | --- | --- | --- |
  | rust-analyzer | yes | yes | yes | yes | yes |
  | Eclipse JDT LS | yes | yes | yes | yes | yes |
  | Ruby LSP | no | no | no | yes | yes |
  | VS Code HTML / CSS | no | no | no | yes | yes |
  | Pyright 1.1.414 (2026-10-16) | no | yes | yes | yes | yes |
  | typescript-language-server 6.0 with TypeScript 5.8 (2026-10-16) | yes | yes | yes (offered to a client that declares call hierarchy, as the reader does) | yes | yes |
  | typescript-go 7.0.2 (2026-10-17) | yes | yes | yes | yes | yes |
  | Kotlin LSP | unverified (not installed on the test machine) | | | | |

- Real-provider tests live in `crates/reader-core/tests/providers.rs` and `crates/reader-gui/src/tests.rs` (`real_*`, ignored);
  they run only against servers named by `READER_<NAME>` / `READER_<NAME>_PATH` or found on `PATH`.

## Status and recovery

Both readers keep the active file's LSP status visible at the start of the footer, including unavailable,
stopped, off, starting, indexing and ready. Click the status, press F2, or choose **Language Server Status…**
from the command menu for server controls. **Server Details…**, also first in the **Language Servers** menu,
opens one scrollable snapshot with full errors, the selected executable, workspace, project settings, versions,
indexing status and provider limits. Its **Copy** button copies the complete, unwrapped report and stays
visible while scrolling; **Close** or Esc dismisses it. Ctrl-C (Cmd-C on macOS) also copies without closing
or copying the source selection; terminal copying uses OSC 52. Arrows, Page Up/Down and Home/End scroll
the report. Reopen it to refresh the snapshot. **Copy Language Server Details** remains in Find Action/F10.

- **Unavailable:** read the specific reason. A missing executable or runtime gets that provider's existing
  installation guidance. Install it yourself, or correct `executable`, `args` and `env` in
  `$XDG_CONFIG_HOME/lenz-reader/providers.json`, then choose **Restart Language Server**. No workspace gets
  **Set Workspace Root…** guidance instead. Large files explain why their server is not started.
- **Stopped:** the full startup or exit error stays in the dialog. Check the named executable and its
  runtime/toolchain from the same environment used to launch Lenz, correct its settings or workspace, then
  restart. Crashed servers are never restarted automatically.
- **Off:** **Enable Language Server for This Folder** activates it. This is an explicit per-folder preference;
  opening the status dialog does not enable it.
- **Starting/indexing:** a supported source starts its installed provider automatically. Initialization has a
  120-second timeout. The footer shows the server's current task and percentage when reported; F2 → **Server Details…** shows the
  full description. Percentages describe that task, not all concurrent work, and servers without percentages
  show their task text. Indexing can produce incomplete results until the server reports completion.
- **Unsupported/no file:** the dialog explains that a supported source file is needed. No installation is
  offered for unsupported document types.

Lenz never installs or upgrades language servers automatically. Their existing provider-specific discovery and
fallback rules still apply. Restart reloads provider settings and checks setup again; actions unavailable in
the current state display their reason. Recovery actions never write source files.

## Project language server settings

Both GUI and terminal readers offer **Exclude Ruby Tooling Gems for This Project** and **Reset LSP Settings
for This Project** in F2 and the Language Servers command group. Open a Ruby source file, wait for its
workspace to resolve, then use the exclusion toggle. It defaults to **On**, excluding Parser, AST, Syntax Tree and
the RuboCop tooling gems measured in the [large-project trial](performance.md#editor-lsp-practices-trial-2026-10-21),
preserving normal project-file coverage. Navigation into those tooling gems becomes unavailable. Existing
custom exclusions and unrelated settings are preserved. Turning it off restores navigation into those tooling
gems without resetting other project settings; turning it on overrides conflicting `includedGems` entries
for this project. Explicit global/project `excludedGems` arrays override the default preset. The action refuses a legacy `.index.yml`, which Ruby LSP would read instead of editor
indexing settings; migrate that file first.

Settings are saved outside the checkout in `$XDG_CONFIG_HOME/lenz-reader/providers.json` (normally
`~/.config/lenz-reader/providers.json`). The key is the **resolved language-server workspace root** shown in
F2, not the browsing folder or the active file's parent. Ruby uses the Gemfile folder unless overridden.
Only that provider/workspace is restarted after a successful save; other projects keep running. Saving
temporarily disables conflicting provider actions. A failed save leaves the running server and file intact.
Reset removes the active provider's complete project override, including any custom project options, and
restores its inherited defaults and global settings.

Any provider can also have project initialization settings edited in this file. For example:

```json
{
  "providers": {
    "ruby-lsp": {
      "settings": { "indexing": { "excludedGems": ["global-tool"] } },
      "projects": {
        "/absolute/path/to/rails-app": {
          "indexing": {
            "excludedGems": ["project-tool"],
            "excludedPatterns": ["generated/**/*.rb"]
          }
        }
      }
    }
  }
}
```

Use the exact absolute workspace path from F2. Precedence is provider defaults, global `settings`, then the
matching `projects` entry. Objects merge recursively; arrays and scalar values replace the inherited value.
In this example the project excludes `project-tool` instead of `global-tool`; include both names to retain
both exclusions. `initialize` and `workspace/configuration` use the same merged settings. After editing,
choose **Restart Language Server**. Executable, arguments and environment remain provider-wide.

To disable the default tooling exclusions globally, set `providers.ruby-lsp.settings.indexing.excludedGems`
to `[]`. A project can override this with its own array, as above. The F2 toggle saves only that workspace.

## Saved declarations and rebuilding

F2 and the Language Servers command group expose the same two controls in GUI and TUI:

- **Reuse Saved Declaration Index: On/Off** defaults to On. It saves Lenz's local declarations outside the
  checkout and reuses them only after checking current source contents. Turning it off stops cache reads/writes
  and rebuilds local declarations normally; it leaves the saved copy available if you turn reuse back on.
  It also disables content-based filtering of language-server notifications after watcher overflow.
- **Rebuild Project Index** drops the current project's saved declarations, refreshes open documents, rebuilds
  local declarations from disk and restarts that project's language servers. It works with cache reuse off too.
  It preserves settings and source files. Disabled servers stay disabled. Other servers' own persistent caches
  are managed by those servers; this action does not erase their workspace/build directories.

Cache reuse is a Reader-wide preference in `$XDG_CONFIG_HOME/lenz-reader/settings.json`:

```json
{ "persistent_index": false }
```

The saved index lives under `$XDG_CACHE_HOME/lenz-reader/declarations-v1` (normally
`~/.cache/lenz-reader/declarations-v1`). It contains declarations, not copies of whole source files. Content
hashes and extractor/dependency identity validate reuse; corrupt or incompatible records fall back to parsing.
The current inventory determines which paths can appear in results. Recent content versions can be reused
after switching branches or renaming files. Rebuilding rejects older in-flight results and cache snapshots.

After watcher overflow, the indexer's existing background reads compare current contents with the last indexed
contents. Files verified unchanged need no duplicate notification to Ruby LSP; changed, added and deleted files
still update the server. Ordinary file and directory events keep their existing behavior. Missing fingerprints,
unreadable files, index limits, servers still initializing and overlapping overflow recovery retain conservative
notifications. This improves recovery from lost events; it does not skip the server's initial project indexing.

Reusable records have a 32 MiB accounted memory budget per active project, shared with the live index where
possible. Completed snapshots are capped at 64 MiB per project; saving prunes the store to eight project
directories. All hashing, loading, writing and removal run in background jobs. A project's first opening and
an explicit rebuild still parse sources; the main cache benefit is reopening unchanged content.

## rust-analyzer (Rust)

| | |
| --- | --- |
| Tested version | **rust-analyzer f8996691e9 (2026-08-30)** (Arch `rust-analyzer 20260831-1`; it reports `rust-analyzer 1 (f8996691e9 2026-08-30)`), Rust 1.98.1 with `rust-src`, Linux, 2026-10-13. Fixtures: `real_rust` in `crates/reader-core/tests/providers.rs` (a two-crate Cargo workspace with lockfile, a CRLF file with a non-BMP character, non-ASCII names, a crate excluded from the workspace), `real_rust_analyzer` in `crates/reader-core/tests/semantics.rs`, and `real_rust_analyzer_through_the_window` in the GUI tests. |
| Launch | `rust-analyzer` from `PATH`; in `$XDG_CONFIG_HOME/lenz-reader/providers.json` under `providers.rust-analyzer`: `executable`, `args`, `env` (added to the reader's environment; `env.PATH` is also where the executable is looked up). Shell startup files are never read. Working directory the workspace root; own process group. `--version` must succeed first. `CARGO_TARGET_DIR` is set to a folder of the workspace's own in the reader's cache, so build-script and proc-macro builds never write the project's `target/`; a `CARGO_TARGET_DIR` in the reader's own environment or in `env` wins, and a `cargo.targetDir` in its `settings` bypasses it (`true` builds into the project's `target/rust-analyzer`). These folders (one per workspace; build output can reach gigabytes) are never cleaned up by the reader. A different launch is a different server. |
| Initialization | `checkOnSave: false`, with `providers.rust-analyzer.settings` merged over it; `workspace/configuration` for `rust-analyzer[.…]` answers from those settings (null without). Client capabilities: UTF-8 and UTF-16 positions, definition links, references, workspace symbols, work-done progress, `experimental.serverStatusNotification`; `applyEdit: false`. |
| Workspace root | As Cargo resolves it: the nearest `rust-project.json` above any manifest; else the nearest `Cargo.toml` and its workspace (`package.workspace`, else the first enclosing `[workspace]` whose `members` globs include it and `exclude` does not); a package no workspace claims is its own root. A manifest change re-resolves roots. An explicit root per browsing root (`providers.rust-analyzer.roots`) overrides it. No manifest: no server, with a prompt to set a root. |
| Readiness | Busy until `experimental/serverStatus` reports quiescent (1.4–1.5 s on the fixture); work-done progress is shown. Initialization alone is not readiness. |
| Operations | Verified: definition across workspace crates (Unicode, CRLF), into the standard library (`rust-src`, "the Rust standard library"), peek; usages (the import and both calls; the declaration left out); workspace symbols; file symbols; a crate excluded from the workspace gets its own server. Definition (one target opens; several, or a peek, list as candidates); usages (`textDocument/references`, declaration excluded; up to 10,000 listed); workspace symbols (`workspace/symbol`, from every running server of the folder, merged and labelled). Answers given while it indexes are marked possibly incomplete. Partial-result streaming is not used. |
| Synchronization | Full-text `didOpen`/`didChange`/`didClose` for the documents tabs hold (large files never), including externally changed content. |
| Dependencies | File targets (Cargo registry/git sources, the standard library) open read-only with their origin shown, served by the project's server (verified: file symbols in a standard-library file, one server). |
| Virtual sources | No retrieval method implemented: non-file targets are reported as unavailable. |
| Edits | `workspace/applyEdit` is declined; the reader never writes project files. rust-analyzer runs build scripts and proc macros (its defaults) with their output in the reader's cache (`CARGO_TARGET_DIR`): with a lockfile present nothing was written into the project and the lockfiles were unchanged (verified). Without a `Cargo.lock`, `cargo metadata` creates one (and may resolve registry dependencies over the network): Cargo's stable tools cannot avoid it. |
| Lifecycle | Started when the first Rust source of a workspace is shown; at most two servers start at once, each given 120 s to initialize; kept warm while any tab holds one of its files or a request is pending; stopped after 10 idle minutes, on project closure or window close (`shutdown`, `exit`, then the process group is killed after 3 s). Never restarted automatically: Restart Language Server is explicit (old answers are discarded). Disable/Enable for a folder is remembered (`disabled`). |
| Memory | The resident size of each server's process group (with its helpers) is sampled every 5 s through `ps`; above 2 GiB combined the reader says so and never stops servers for it. On macOS this is resident size, not physical footprint (unverified). |
| Measured (release, fixture, three runs; warm: 100 samples after 5) | Ready (quiescent) 1.4–1.5 s. First answers once ready: definition 175–180 ms, usages 2.1–2.2 ms. Warm definition to target shown: median 2.1 ms, p95 3.2 ms; usages listed: median 2.1 ms, p95 3.2 ms (the harness's polling floor). Memory: 530–634 MiB, helpers (proc-macro server) 73 MiB. |
| Platforms | Linux verified with the real server through the session and GPUI's test platform (Ctrl-click, Alt+F12, Shift+F12, Ctrl+T then Tab); macOS unverified. |
| Known limits | Definitions inside macro expansions or generated code may point at sources rust-analyzer cannot show. |

## Pyright (Python)

| | |
| --- | --- |
| Tested version | **Pyright 1.1.414** (`pyright --version`), Node.js 26.8.1, CPython 3.14 standard library, Linux, 2026-10-13. Fixture: `real_pyright` in `crates/reader-core/tests/providers.rs` (written by the test, pinned in its source). |
| Launch | `pyright-langserver --stdio` from `PATH`, or `executable`/`args`/`env` under `providers.pyright`. It needs Node.js on the `PATH` it runs with (`env.PATH`); the reader never reads shell startup files, so a version manager's shims need an explicit `env.PATH`. Its version is read from the npm package that holds the executable (`package.json`, symlinks resolved); the language server has no version flag and names none when initialized, and the reader never runs the `pyright` command (the PyPI wrapper of that name downloads Node.js and Pyright when it first runs). An unreadable version is shown as unknown and untested; Pyright still starts. Evidence recipe: `READER_PYRIGHT=<node 26.8.1>/bin/pyright-langserver READER_PYRIGHT_PATH=<node 26.8.1>/bin:/usr/bin:/bin` (a non-default mise Node install, set as `executable`/`env.PATH`). |
| Initialization | No options; `workspace/configuration` sections `python[.…]` answer from `providers.pyright.settings` (null without), `pyright` with null. Hierarchical document symbols advertised. |
| Workspace root | The nearest `pyrightconfig.json`, else the nearest `pyproject.toml`/`setup.py`/`setup.cfg`, never above the browsing root; else the browsing root; else the file's folder. A nested project gets its own server; a `pyrightconfig.json` higher up wins over a nearer project file (a project nested inside one is part of it unless its root is set explicitly). |
| Readiness | Pyright reports progress but no readiness status; for 10 s after it initializes its answers are marked possibly incomplete (it answers workspace symbols with nothing until its workspace has loaded). |
| Operations | Definition (functions, classes, methods, standard library through bundled stubs and the interpreter's own sources, several candidates for overloads), usages (complete when asked at the declaration; asked at a use, that use is left out), workspace symbols, file symbols (nested; parameters are listed). |
| Dependencies | File targets: Pyright's bundled typeshed stubs ("Pyright's bundled type stubs"), installed packages (`site-packages`), the standard library. No virtual sources. |
| Limits | Static analysis only (no dynamic attributes or monkey patching); unresolved imports are not followed; files under hidden folders are outside its workspace by default (no workspace symbols for them). |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 2.1 ms, p95 3.2 ms. Usages listed: median 3.2 ms, p95 3.2 ms. Server memory 171 MiB, no helpers. |
| Platforms | Linux verified through the session and through GPUI's test platform (no private display: Xvfb is absent); macOS unverified. |

## typescript-language-server (TypeScript, TSX, JavaScript)

| | |
| --- | --- |
| Tested version | **typescript-language-server 6.0.1** with **TypeScript 6.0.3** (`tsserver.js` from the VS Code install, named by the user's `tsserver.path` setting), Node.js 26.10.0, Linux, 2026-10-13. Fixture: `real_typescript` in `crates/reader-core/tests/providers.rs`, pinned in its source, with parts per language: TypeScript (`src/geometry.ts`, `src/app.ts` with CRLF and a non-BMP character, `src/extra.ts`), TSX (`src/view.tsx`), JavaScript (`src/util.js`, `src/main.js`), a nested package (`packages/ui`) and an `@types` package. |
| Launch | `typescript-language-server --stdio` from `PATH`, or `executable`/`args`/`env` under `providers.typescript-language-server`; needs Node.js on its `PATH`. Version from `--version`. It uses the project's `node_modules/typescript`, else the user's `settings.tsserver.path` (passed in `initializationOptions`); TypeScript 7 (the native compiler) has no tsserver, so with only that installed it fails to initialize and the reader shows its message. The reader never chooses a TypeScript for the user. |
| Languages | TypeScript (`typescript`), TSX (`typescriptreact`), JavaScript (`javascript`, `.jsx` as `javascriptreact`): one owner; TSX never goes to the HTML server. |
| Workspace root | The outermost `tsconfig.json`/`jsconfig.json`/`package.json` under the browsing root, else the browsing root (without one: the nearest, else the file's folder). Nested configurations share that server (tsserver resolves each file's project itself). |
| Readiness | No readiness status: for 10 s after it initializes, answers are marked possibly incomplete, and a single definition jumped to then carries a notice. Early definitions can stop at the import binding (seen at 0.3 s) until tsserver has loaded the project. |
| Operations | Definition: TypeScript → TypeScript, TypeScript → JavaScript, TSX → TypeScript, JavaScript → JavaScript, into TypeScript's library declarations and an `@types` package. Usages from the declaration: TypeScript (TS, TSX, nested package) and JavaScript (JS and TS users), imports included. Workspace symbols (rows show the declaration's line); file symbols (nested; constructors and parameter properties listed). A replaced question and a stale result (file changed after listing) behave as for every provider. |
| Dependencies | File targets: TypeScript's library declarations (`lib.*.d.ts`), `@types` packages, other `node_modules`. No virtual sources. |
| Limits | Navigation follows TypeScript's project model; plain JavaScript is resolved by inference; a TypeScript with tsserver is required (where only TypeScript 7 is found, typescript-go serves instead, below); TypeScript 6 includes no `@types` package unless `compilerOptions.types` names it. Without a browsing root, a file's server is rooted at its nearest project file (so one monorepo opened file by file can get more than one server). |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 2.1 ms, p95 3.2 ms. Usages listed: median 2.1–3.2 ms, p95 3.2 ms. Memory: server 76–78 MiB, tsserver (its helper) 656–675 MiB. |
| Platforms | Linux verified through the session and GPUI's test platform (no Xvfb); macOS unverified. |

## typescript-go (TypeScript 7: TypeScript, TSX, JavaScript)

| | |
| --- | --- |
| Tested version | **typescript-go 7.0.2** (`tsc --lsp --stdio` of the npm `typescript` 7.0.2, global under Node.js 26.10.0), Linux, 2026-10-17. Fixtures: `real_typescript_go` and `real_typescript_7_installation_chooses_typescript_go` in `crates/reader-core/tests/providers.rs` (the TypeScript fixture above); the choice and launch also with a fake server and in unit tests. |
| Choice | Both TypeScript servers serve the same languages; which one serves a workspace is decided when its root is resolved (off the UI thread, reading files only): typescript-language-server when its settings name a `tsserver.path`/`fallbackPath`, when the project's `node_modules/typescript` (at the root or above) has `lib/tsserver.js`, or, with no project TypeScript, when a TypeScript with tsserver is installed beside typescript-language-server (what it falls back to). Otherwise typescript-go, if a TypeScript 7 `tsc` can run (else typescript-language-server, which says what is missing). One choice per root: a folder resolved later joins the server already serving its root; Restart Language Server chooses again (after TypeScript 7 is installed or removed). A `tsserver.path` that is not there is ignored, as typescript-language-server ignores it. Per-folder choices (Disable for this folder, a workspace root) are typescript-language-server's entry in the settings and cover both. Global `tsc` shims that are scripts (pnpm global, Volta) are not followed to their package: name the executable. |
| Launch | `tsc --lsp --stdio`: the project's own `node_modules/.bin/tsc` (at the root or above) when that project's `typescript` is version 7 or newer, else `tsc` on `PATH` when its npm package is; or `executable`/`args`/`env` under `providers.typescript-go` (a named executable is used as it is). The npm `tsc` is a Node.js script that runs the native compiler: Node.js on its `PATH`. Version from its `initialize` answer (`typescript-go 7.0.2`). |
| Workspace root | As typescript-language-server's. |
| Readiness | No readiness status: the 10 s "may still be loading" marking. On the fixture it answered the first definition at once. |
| Operations | Verified: definition TS → TS (Unicode, CRLF), TS → JS, TSX → TS, JS → JS, into an `@types` package and into TypeScript's library declarations; usages (TS, TSX, nested package); workspace and file symbols; one server for nested configurations, every file kind and a dependency; an external edit; files unchanged. Implementation, type definition, call hierarchy, hover and highlights are offered (its `initialize` capabilities); no type hierarchy (the reader's syntax-based one is used). Inlay hints and semantic tokens are offered; not verified against the reader's decorations. |
| Dependencies | TypeScript 7's library declarations live in its platform package (`node_modules/@typescript/typescript-<platform>/lib/lib.*.d.ts`): shown as "TypeScript's library declarations". |
| Limits | File symbols include a constructor with an empty name (as the server reports it). |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 2.2 ms, p95 3.2 ms. Memory: 69 MiB, no helper process (typescript-language-server with TypeScript 5.8: about 80 MiB plus 625–675 MiB for tsserver). |
| Platforms | Linux verified through the session; macOS unverified. |

## Eclipse JDT LS (Java)

| | |
| --- | --- |
| Tested version | **JDT Language Server 1.61.0-SNAPSHOT** (its `initialize` answer; Arch package `jdtls 1.61.0-1`), its Python launcher on Python 3.14, Java from `JAVA_HOME` = Temurin 21.0.2 (sdkman), Linux, 2026-10-13. Fixture: `real_jdtls` in `crates/reader-core/tests/providers.rs` (a plain source tree with Unicode identifiers and a CRLF file, pinned in its source). |
| Launch | `jdtls --jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false -data <reader cache>/jdtls/<project>-<hash>` (`$XDG_CACHE_HOME/lenz-reader`): a workspace-data folder per project root and launch, so projects (even with the same folder name) and launch configurations never share one; jdtls's own default is keyed by the folder's name alone. `executable`/`args`/`env` (PATH, JAVA_HOME) under `providers.jdtls`, where `args` add to the fixed metadata property (never replace it; it is not part of the data folder's identity, so folders made before it keep being used); its launcher needs Python 3 and Java 21+. Version from its `initialize` answer (the launcher has no version flag). |
| Initialization | `extendedClientCapabilities.classFileContentsSupport: true` and `settings.java.autobuild.enabled: false` (a reader never builds), with `providers.jdtls.settings` merged over them object by object (a user's `settings.java…` keeps the others); `workspace/configuration` for `java[.…]` from those settings. |
| Workspace root | The outermost `pom.xml`/`build.gradle[.kts]`/`settings.gradle[.kts]`/`.project` under the browsing root (it imports nested modules itself), else the browsing root; without one, the nearest, followed up through parent Maven/Gradle builds. |
| Readiness | `language/status`: busy from `Starting` until `ServiceReady` (1.4 s on the fixture); `Error` messages are shown; build/import progress is shown. |
| Operations | Definition (within the project; JDK classes as retrieved source), usages (from the declaration), workspace symbols (types: a class name finds it; a method name, `twice`, found nothing; rows show the declaration's line), file symbols (nested; the package declaration is listed). |
| Virtual sources | `jdt://` targets (JDK and library classes) are retrieved through `java/classFileContents`, from the server session that named them, into a kept read-only tab ("PrintStream.java (Eclipse JDT LS)") with the URI in the status bar; never written, watched, reloaded or sent back to a server; after that session ends the tab says so; once closed it is not held. Several virtual targets only: the first is retrieved and the count said; virtual targets beside file targets are counted, not listed. Peek on a virtual target opens the retrieved tab (said so): there is no file to preview. |
| Side effects | A folder without a build file: nothing written into the project (verified). A Maven project (verified with the Kotlin fixture's `pom.xml`, `real_kotlin_with_jdtls`): imported by JDT LS through m2e, which resolves plugins and dependencies through `~/.m2` (and the network for anything missing); by default it also wrote `.project`, `.classpath`, `.settings/` and built `target/classes` into the project, so the reader launches it with `--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false` (Eclipse metadata kept in its workspace-data folder; a JVM property because it is read at startup, as VS Code's Java extension passes it) and `java.autobuild.enabled: false` in its settings: nothing written into the project (verified). Gradle import untested. Workspace-data folders stay in the reader's cache (hundreds of MiB per imported project) and are not cleaned up; two reader windows on the same project share one folder, which Eclipse locks (the second server fails, with its message). |
| Limits | Kotlin files are not part of its index (its usages say they leave out Kotlin callers when the project has Kotlin files); Gradle import untested; definitions from inside retrieved source are not available. |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 58.9 ms, p95 61.2 ms (the server's own time; other providers answer in 2–3 ms). Usages listed: median 5.3 ms, p95 7.4 ms. Memory: 2.1–2.2 GiB (`-Xms1G` and its heap; above the reader's 2 GiB notice on its own). |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click into retrieved JDK source; no Xvfb); macOS unverified. |

## VS Code HTML and CSS servers (HTML, CSS)

| | |
| --- | --- |
| Tested version | The servers built into **VS Code 1.141.0** (Arch `visual-studio-code-bin 1.141.0-1`: `html-language-features` and `css-language-features` 10.0.0), run with Node.js 26.10.0, Linux, 2026-10-13. Fixtures: `real_html` and `real_css` in `crates/reader-core/tests/providers.rs` (a page with an inline `<style>` and `<script>`, a non-BMP character and non-ASCII names; a CRLF stylesheet with a non-ASCII custom property), pinned in their source; `real_html_and_css_through_the_window` in the GUI tests. `vscode-langservers-extracted` (npm) is the documented alternative, untested. |
| Launch | `vscode-html-language-server --stdio` / `vscode-css-language-server --stdio` from `PATH` (npm `vscode-langservers-extracted`; version from its `package.json`). When those are not on `PATH` and the settings name no `executable`, the server an installed VS Code ships (`/usr/share/code`, `/opt/visual-studio-code`, `/usr/lib/code`, the snap, the macOS app) runs as `node <script> --stdio`, with `node` from the launch's `PATH`; its version is the VS Code version (`package.json` above the extensions folder), read, never run. Without Node.js it is missing, said so ("needs Node.js"). Other installs (Flatpak, `~/Applications` on macOS, a tarball) are not searched: set `executable` (`node`) and `args` (the script, `--stdio`). The choice between PATH and VS Code is made when the server starts: a later install or VS Code update is picked up by Restart Language Server. The script runs with the same trust as launching that VS Code (a system install location; no ownership check). `executable`/`args`/`env` under `providers.vscode-html` / `providers.vscode-css`. |
| Initialization | `provideFormatter: false`; HTML also `embeddedLanguages: { css: true, javascript: true }`; the user's `settings` merged over them, answered for `html[.…]` / `css[.…]`. |
| Languages | HTML (`html`), CSS (`css`). TSX, JSX and JavaScript files stay with the TypeScript provider. SCSS/Less are not detected as CSS by the shared highlighter and get no server. |
| Workspace root | No project model: the browsing root the file is in, else its folder. One server per browsing root and language. |
| Operations | CSS: definition and usages of custom properties and `@keyframes` within the same stylesheet (Unicode and CRLF positions verified); file symbols (rules, `:root`, keyframes). HTML: definition and usages inside the page's own `<script>` (functions, variables) and `<style>` (custom properties); file symbols (its elements and style rules). Both include the declaration in usages (they ignore `includeDeclaration: false`). |
| Not provided | A class or id in HTML is not followed to a stylesheet, nor a selector to markup, templates, frameworks or other files: the definition says it found none (no text match stands in). No workspace symbols: the command is disabled with "The language servers running for this folder do not provide workspace symbols; find declarations instead" (not an empty result); syntactic declarations and text search stay available. |
| Dependencies | `node_modules` files show "an npm dependency"; no virtual sources. |
| Side effects | Nothing written into the project (verified: the fixture's files are unchanged afterwards); formatting is off. |
| Measured (release, fixture, 100 samples after 5) | Ready 0.1 s (CSS), 0.2 s (HTML). Warm definition to target shown: median 2.1 ms, p95 2.1 ms (both): the test harness's polling floor (about 2 ms, as for typescript-language-server), so the servers answer at least that fast. Memory: CSS 80–88 MiB, HTML 242–243 MiB (it embeds TypeScript's language service for scripts). Raw samples: `READER_PERF` lines of `real_css` / `real_html`. |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click, Shift+F12, the F10 reason, an external edit, a workspace root set for the page, Back; no Xvfb); macOS unverified (the app path is listed, not exercised). |

## Ruby LSP (Ruby)

| | |
| --- | --- |
| Tested version | **ruby-lsp 0.26.11** (`gem install --user-install ruby-lsp`), Ruby 3.4.10 (Arch), RubyGems 4.0.15, Bundler 4.0.15, Linux, 2026-10-13. Fixture: `real_ruby` in `crates/reader-core/tests/providers.rs` (a CRLF file with a non-BMP character, non-ASCII class and method names, a nested bundle, a bundle locked offline to the installed `thor` 1.4.0), pinned in its source; `real_ruby_conflicting_addon_versions` (two installed versions of an add-on gem that activate conflicting versions of another gem); `real_ruby_through_the_window` in the GUI tests. |
| Launch recipe | `ruby-lsp` from PATH (or `executable`/`env` under `providers.ruby-lsp`) inside a bundle Bundler sets up (`RUBYOPT=-rbundler/setup`, as editors' `bundle exec`): the project's own when its lock lists `ruby-lsp`; otherwise one the reader composes in its cache (`$XDG_CACHE_HOME/lenz-reader/ruby-lsp/bundles/<project>-<hash>/`): a `Gemfile` that evaluates the project's (`eval_gemfile`) and adds `ruby-lsp` (and `ruby-lsp-rails` for a Rails app when it is installed), with a copy of the project's lock (relative path sources made absolute), resolved by `bundle lock --local` (the `bundle` beside `ruby-lsp`, else on PATH) from installed gems only and redone only when the Gemfile it writes, the project's lock or `PATH`/`GEM_HOME`/`GEM_PATH` change. The project's `.bundle` settings apply (`BUNDLE_APP_CONFIG`; a relative `BUNDLE_PATH` from the project). A failed lock stops the launch with Bundler's message. Outside a bundle (0.0.3), Ruby LSP loads the add-ons of every installed version of every gem: with an RVM gem set holding two RuboCop versions, `initialized` failed with `Gem::ConflictError` and indexing stayed at 0%. Its default launch (no `BUNDLE_GEMFILE`) is not compliant: it writes `.ruby-lsp/` (a composed Gemfile, lockfile and markers) into the project, runs `bundle install` (network) and every four hours `bundle update ruby-lsp debug prism` (an upgrade of the server); the reader never uses it. A user `env.BUNDLE_GEMFILE` or `env.RUBYOPT` wins. Version from `ruby-lsp --version` (it exits before any bundle work). |
| Initialization | `formatter: "none"`, `linters: []` (no RuboCop: it would load the project's configuration and plugins), and the [tooling-gem exclusion preset](#project-language-server-settings); the user's global/project `settings` are merged over them. Positions: UTF-16 only (0.26.11 answers index positions in characters even with UTF-8 negotiated: a non-ASCII name in a file it does not hold open ended a byte short); characters outside the BMP in such names would still be off. |
| Workspace root | The nearest folder with a `Gemfile`/`gems.rb` under the browsing root (each bundle is its own project: engines, nested apps), else the browsing root; without one, the nearest such folder, else the file's. Override per browsing root (verified through the command menu's modal); an overridden root inside a bundle keeps that bundle's Gemfile (the nearest at or above the root). |
| Readiness | No readiness status: for 10 s after it initializes, answers are marked possibly incomplete; on the fixture it indexed within 0.2–0.3 s, RBS core signatures and gems shortly after. |
| Operations | Definition: classes and modules (constants), constant assignments (the whole assignment is selected), methods (by name), across files; into RBS core signatures (`File` → `file.rbs`) and into installed gems (`Thor` is reopened in many of its files: candidates). Usages (from a use or the declaration; the declaration is included: it ignores `includeDeclaration: false`): a constant's (`AREA_UNIT`: its use and assignment) separately from a method's (`área`: every call by name, any receiver, and its `def`). Workspace symbols: fuzzy over qualified names (`Shapes::Círculo` finds it; a short name inside a long namespace can fall below its threshold); file symbols (nested). Never text matches. |
| Dependencies | Installed gems ("an installed gem", recognized by their installation folders: user and system gem paths, version managers, `vendor/bundle`; a project's own `gems/` folder is not one), RBS core signatures ("RBS core signatures"), the standard library ("the Ruby standard library"), as files; no virtual sources. Gems are indexed from the bundle, at their locked versions. Every locked gem must be installed: otherwise the reader's `bundle lock --local` (or Bundler's setup) fails and the launch stops with Bundler's message; run `bundle install` in the project (the reader never installs gems). Lenz 0.0.3 and earlier skipped uninstalled gems instead. |
| Side effects | Bundler evaluates the project's Gemfile, and Ruby LSP loads the add-ons of gems in the bundle and in the project (`**/ruby_lsp/**/addon.rb`): project code runs, as in any editor running Ruby LSP; the Rails add-on loads the application. It writes nothing into the project (verified on the fixtures), with one exception: a `.ruby-lsp/bundle_env` left by an editor's launch is read and deleted at startup. The reader writes only its composed bundle in its cache; nothing is installed or downloaded. |
| Limits | Static index: dynamic definitions, `method_missing` and metaprogramming are not followed; method usages are by name; external edits reach it through the reader's changed-file reports (verified: a moved class is found at its new line). |
| Measured (release, fixture, 100 samples after 5) | Ready 0.2–0.3 s. Warm definition to target shown: median 2.1 ms, p95 3.2 ms (the harness's polling floor is about 2 ms). Memory: 109–117 MiB (three runs; raw samples in the code-reader build evidence, `real-ruby-3x.txt`). |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click, Back, Shift+F12, the root modal; no Xvfb); macOS unverified. |

### Rails setup and large projects

Use Ruby LSP; **0.26.11** is the version tested above. Activate the project's Ruby version in your terminal,
then launch Lenz from the Rails root (the folder containing `Gemfile`):

```sh
cd /path/to/rails-app
ruby -v
ruby-lsp --version
lenz .
```

Opening a `.rb` file starts the server. F2 shows its workspace, executable and version. Lenz inherits the
launch environment and does not read shell startup files. If a Dock launch cannot find your Ruby, set an
absolute `executable` and the matching `env.PATH`/gem environment in `providers.ruby-lsp` in
`~/.config/lenz-reader/providers.json` (or `$XDG_CONFIG_HOME/lenz-reader/providers.json`). With rbenv, for example,
the executable can be `/Users/your-name/.rbenv/shims/ruby-lsp`;
[Ruby LSP's editor instructions](https://shopify.github.io/ruby-lsp/editors.html) explain version-manager setup.

For Rails-specific navigation, install `ruby-lsp-rails` with the same Ruby. Lenz adds it to the bundle it
composes for a Rails app only when it is already installed; it is not installed automatically as in the standard
Ruby LSP launch. The [Rails add-on](https://shopify.github.io/ruby-lsp/rails-add-on) loads the Rails application
for runtime introspection. Its VS Code-specific controls are not Reader features.

For large projects, merge exclusions for generated or copied Ruby sources you do not navigate into the existing
provider settings, then choose **Restart Language Server**. For example (replace these paths with your own):

```json
{
  "providers": {
    "ruby-lsp": {
      "settings": {
        "indexing": {
          "excludedPatterns": ["generated/**/*.rb", "vendor/legacy/**/*.rb"]
        }
      }
    }
  }
}
```

These are Ruby LSP's [indexing options](https://shopify.github.io/ruby-lsp/editors.html#all-initialization-options),
separate from Lenz's local declaration index. Use `**/*.rb` to cover nested Ruby files; a trailing `/**` alone
does not cover arbitrary nested paths with this server's matching flags. Ruby LSP 0.26.11 already skips
top-level `tmp`, `node_modules` and `sorbet`, common test/spec filenames, fixtures and the configured bundle path
during project indexing. Repeating those defaults may save nothing. `includedPatterns` adds paths to the
defaults; it does not restrict the index to an allowlist.

Choose only folders whose symbols you do not need. Installed gems are indexed separately; `excludedGems` can
omit particular gems at the cost of their navigation coverage, even when linting is disabled. Global provider
settings apply to all Ruby workspaces; use a [project override](#project-language-server-settings) to scope
exclusions to one workspace.
An existing `.index.yml` takes precedence over these options in Ruby LSP 0.26.11. Indexing progress improves
visibility; it does not shorten the server's indexing work. The [large Rails experiment](performance.md#large-rails-indexing-experiment-2026-10-21)
compares exclusions with competing Lenz work; it does not enable exclusions by default.

## Kotlin LSP (Kotlin)

| | |
| --- | --- |
| Tested version | **Kotlin LSP 263.6379.0-EAP** (JetBrains' standalone archive `kotlin-server-263.6379.0.tar.gz`, SHA-256 checked against its published checksum; it bundles its own Java runtime; `bin/intellij-server`), Linux, 2026-10-13. Fixtures: `real_kotlin` and `real_kotlin_with_jdtls` in `crates/reader-core/tests/providers.rs` (a mixed Maven project: Kotlin 2.2.21 with Kotlin and Java sources, a CRLF file with a non-BMP character, non-ASCII names), pinned in their source; `real_kotlin_through_the_window` in the GUI tests. They run only when Kotlin 2.2.21 is already in `~/.m2`. |
| Launch | `kotlin-lsp --stdio --data-sharing=none --system-path <reader cache>/kotlin-lsp/<project>-<hash>` (`kotlin-lsp` being its `bin/intellij-server`, or `executable` under `providers.kotlin-lsp`; `args` add to the fixed `--stdio --data-sharing=none`): its caches and indexes per project and launch in the reader's cache; no usage data sent. Version from `--version` (which leaves a small log in `~/.cache/JetBrains/kotlin-server2026.3`). No EULA prompt or acceptance record was involved with this build, and its license check reported the build's own EAP licence ("This build does not require a license"); the reader never accepts terms or signs in for the user. Its licence and terms beyond what it reported are unverified. |
| Languages | Kotlin (`kotlin`). Java files stay with JDT LS. |
| Workspace root | The outermost Maven/Gradle build under the browsing root (as for JDT LS), else the browsing root; overridable. A folder without a build is not imported ("no supported build system"): definitions within a file only, nothing across files (verified). |
| Readiness | Its Maven/Gradle import reports progress (shown as indexing: "Importing Maven"); ready in 1.7–1.8 s on the fixture, the index a few seconds later. Reopening files every half second during the import (an early GUI test's retry loop) made it start Maven again and again without finishing; normal browsing was not affected in the tests. |
| Operations | Definition: Kotlin to Kotlin (Unicode, CRLF), Kotlin to Java sources of the same project, also in a Kotlin-only project and after Restart Language Server. Usages of a Kotlin declaration include Java callers (its index has the project's Java sources; verified). File symbols. Workspace symbols are offered; not exercised through the reader (a direct probe listed library classes inside jars first). |
| Not provided | Java to Kotlin: Java files belong to JDT LS, which does not index Kotlin, so it finds nothing (said, with its limits; verified), and its usages say they leave out Kotlin callers (verified). The two servers' answers are never combined. Library sources inside jars (the Kotlin standard library: `jar:` targets): reported as source unavailable, not retrieved. |
| Side effects | It imports by running Maven (`mvn` from PATH) or Gradle: build plugins run, and missing artifacts are downloaded into `~/.m2` (seen: the standard library's sources and javadoc, and its import plugin's dependencies, kotlinx-serialization 1.9.0 and kotlin-stdlib 2.2.0, during the first imports). Nothing written into the project (verified, also alongside JDT LS). |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 38–39 ms, p95 53–65 ms. Memory: 1.5–1.6 GiB (with JDT LS's 2.1–2.2 GiB the reader's 2 GiB notice shows). |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click to Kotlin and to Java, Back, Shift+F12; no Xvfb); macOS unverified. Gradle projects untested (no offline Gradle Kotlin setup here); generated sources, root overrides and cancellation during an import untested with the real server (shared behaviour, covered by fake-server tests). Warm latency from 100 samples, as for the other providers (the spec's 1,000-sample methodology is not met for any provider). |

## Repairing a Java server runtime in the reader

In either frontend, open a Java file, press **F2** (Language Server Status), then choose **Choose Language Server Runtime…**.
Select a detected Java executable, or enter/paste an executable path or JDK home folder and choose **Save entered
path and restart**. Lenz runs `java -version` with a three-second timeout, saves the JDT LS `--java-executable`
argument in `providers.json`, and restarts enabled Java servers. Other provider settings and disabled states are
preserved. Invalid paths, malformed settings and save errors are shown in the dialog without replacing the
existing settings. Escape before saving cancels; a save already submitted continues after dismissal.

**Restore automatic Java selection and restart** removes the override. This does not install Java or JDT LS, or
verify the runtime meets the installed JDT LS version's requirements: check the LSP status after restarting and
use **Copy Language Server Details** for the complete failure report, including configured arguments. Missing
servers still require installation; disabled servers can be enabled from the same status dialog.


## Version context and Pyright runtime repair

**Server Details…** and **Copy Language Server Details** include the detected server version (or explicitly unknown) and the
version tested with Lenz. A different version is unverified, not automatically incompatible. Failed launches
retain up to 8 KiB of stderr, keeping the beginning and end with an omission marker for larger logs, and show
the resolved executable. Pyright/Java failures also probe the selected Node.js/Java executable and version in
the server's environment and workspace, with a three-second limit. Pyright's declared Node.js requirement is
shown when its installed package can be read. Missing-server setup instructions remain in the same dialog.

For Pyright, open a Python file and use **F2 → Choose Language Server Runtime…**. Select an existing Node.js
installation containing Pyright, or paste its `bin/node` executable or installation folder. Validation runs
`node --version`; saving records only `providers.pyright.node` and restarts enabled Pyright servers. Reader
then prefers the matching `pyright-langserver` beside that Node installation and prepends its bin folder to
that provider's PATH. Custom executables, arguments, environment, other providers and disabled states are
preserved. **Restore automatic Node.js selection and restart** removes only this override.

A mise message such as “No version is set for shim: pyright-langserver” means the selected mise installation
cannot provide that shim; it does not prove Pyright is incompatible with the selected Node version. Choose an
installation where Pyright already exists, then check the new status. Lenz does not change mise's global
version, read shell startup files, install tools or silently retry another runtime. Java repair still changes
only JDT LS's launcher argument as described above. Other providers expose their version, failure and setup
instructions; they do not yet have runtime pickers.
