# Semantic provider compatibility

What each built-in provider is tested to do, populated from implementation tests only. A row without a tested
version has not been exercised against the real server; the reader still runs an installed one, with an
"untested version" notice. Servers are installed only through an explicit managed install or your own setup; nothing installs or upgrades automatically.

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
  | yaml-language-server 2.0.0 (2026-10-26) | no | no | no | yes | no |
  | bash-language-server 5.8.1 (2026-10-26) | no | no | no | yes | yes |

- Real-provider tests live in `crates/code-core/tests/providers.rs` and `crates/code-gui/src/tests.rs` (`real_*`, ignored);
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
  installation guidance. Choose **Install Language Server…** for a command preview and an explicit managed npm install where supported, or install it yourself. Correct `executable`, `args` and `env` in
  `$XDG_CONFIG_HOME/lenz-code/providers.json`, then choose **Restart Language Server**. No workspace gets
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

## Explicit managed installation

F2 → **Install Language Server…** in either frontend opens a focused preview. **Start Installation** (`S` or
Enter) starts it; opening the dialog alone does nothing. **Cancel Installation** (`C`) kills the owned installer
process group, including its children. Escape or Close also cancels. The dialog shows phases and up to 32 KiB of
sanitized installer output; output may be omitted if the completion queue is busy. **Server Details…** (`D`)
keeps the complete original diagnosis available, and Ctrl-C copies the preview/output. The editor remains responsive.

The curated npm recipes install outside projects, below `servers/<provider-id>/` beside `providers.json`.
The preview shows the resolved Node/npm executable paths, exact argument list, target directory and disabled
package scripts. npm's cache stays there too. Lenz invokes the npm JavaScript CLI with the provider's selected
Node explicitly, using its configured PATH (including Pyright's selected runtime); shell startup files are not read.
Node must be named `node`, as in the runtime chooser. npm shims that are shell wrappers are not supported by this
launcher: use the installed npm JavaScript CLI on the provider PATH. npm checks package engine requirements strictly.

| Provider | Package selected by Install |
| --- | --- |
| Shell | `bash-language-server@5.8.1` |
| Python | `pyright@1.1.414` |
| YAML | `yaml-language-server@2.0.0` |
| TypeScript / JavaScript | `typescript-language-server@6.0.1` and TypeScript 6 |
| HTML, CSS | `vscode-langservers-extracted@4.10.0` (each provider has its own managed directory) |

Node.js and npm must already be installed. Missing managers, custom executable overrides, unsupported platforms
and providers without a recipe show their next setup action and refuse Start. Ruby, Go, Rust, Java, Kotlin and
TypeScript 7 use the existing manual setup guidance; Lenz does not invoke gem, go, rustup or system package managers.
ShellCheck and shfmt remain separate optional tools and are not installed by this action. No sudo, global npm
installation, project manifest edits or dependency lifecycle scripts are used. Top-level server versions are pinned;
npm resolves their dependencies normally, without a lockfile. Network/registry failures and runtime errors remain visible.

After npm succeeds, Lenz verifies the package's executable stays inside its managed directory, reads the package
identity/version and checks the launcher with the selected Node runtime. Only then does it atomically save the
executable and runtime PATH in `providers.json`, preserving provider arguments, settings, unknown fields and unrelated
providers. A changed target-provider configuration refuses the save. Only the installed provider is reloaded; switching
tabs never retargets the operation, and a changed active file before Start requires reopening the preview.
Cancelling before persistence keeps the previous provider configuration; partial downloads may remain in the managed
directory. Close/reopen waits for a cancelled worker to finish before another install can start. A filesystem lock also excludes independent windows/processes using the same managed prefix, and remains held through validation and settings persistence. Contention reports that installation is already running before a second subprocess starts; cancellation/failure releases the lock. Validation uses fake
package managers and real subprocess/cancellation paths; live downloads and native macOS installation are unverified.

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

Settings are saved outside the checkout in `$XDG_CONFIG_HOME/lenz-code/providers.json` (normally
`~/.config/lenz-code/providers.json`). The key is the **resolved language-server workspace root** shown in
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

Cache reuse is a preference for Lenz Code in `$XDG_CONFIG_HOME/lenz-code/settings.json`:

```json
{ "persistent_index": false }
```

The saved index lives under `$XDG_CACHE_HOME/lenz-code/declarations-v1` (normally
`~/.cache/lenz-code/declarations-v1`). It contains declarations, not copies of whole source files. Content
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
| Tested version | **rust-analyzer f8996691e9 (2026-08-30)** (Arch `rust-analyzer 20260831-1`; it reports `rust-analyzer 1 (f8996691e9 2026-08-30)`), Rust 1.98.1 with `rust-src`, Linux, 2026-10-13. Fixtures: `real_rust` in `crates/code-core/tests/providers.rs` (a two-crate Cargo workspace with lockfile, a CRLF file with a non-BMP character, non-ASCII names, a crate excluded from the workspace), `real_rust_analyzer` in `crates/code-core/tests/semantics.rs`, and `real_rust_analyzer_through_the_window` in the GUI tests. |
| Launch | `rust-analyzer` from `PATH`; in `$XDG_CONFIG_HOME/lenz-code/providers.json` under `providers.rust-analyzer`: `executable`, `args`, `env` (added to the reader's environment; `env.PATH` is also where the executable is looked up). Shell startup files are never read. Working directory the workspace root; own process group. `--version` must succeed first. `CARGO_TARGET_DIR` is set to a folder of the workspace's own in the reader's cache, so build-script and proc-macro builds never write the project's `target/`; a `CARGO_TARGET_DIR` in the reader's own environment or in `env` wins, and a `cargo.targetDir` in its `settings` bypasses it (`true` builds into the project's `target/rust-analyzer`). These folders (one per workspace; build output can reach gigabytes) are never cleaned up by the reader. A different launch is a different server. |
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
| Editing (ticket 46) | Verified on rust-analyzer 1.97.0 (rustup component, with rust-src and rustfmt), Linux, 2026-10-25, with `real_rust_editing` and `real_rust_completion` in `crates/code-core/tests/providers.rs` and `real_rust_analyzer_fix_and_format_match_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`: the "Import `std::collections::HashMap`" assist as one Undo step (Alt+Enter in both adapters), type-aware members of a `HashMap<u32, u32>`, completing `HashSet` with its merged `use std::collections::{HashMap, HashSet};` as one Undo step, parameter information for `insert` with the active parameter, rustfmt through Format Document (Ctrl+Alt+L in both adapters), and a format answered after an edit refused. Problems are rust-analyzer's own: it answers `textDocument/diagnostic` (empty while loading, then `workspace/diagnostic/refresh`, which Lenz answers by asking again). Lenz starts it with `checkOnSave: false`, so compiler errors from `cargo check` (an undeclared `HashMap`, for one) are not shown. |

## gopls (Go)

| | |
| --- | --- |
| Tested version | **gopls v0.24.0** (`go install golang.org/x/tools/gopls@latest`), Go 1.24.7, Linux, 2026-10-26. Fixtures: `real_gopls_editing` and `a_missing_gopls_says_how_to_install_it_and_editing_goes_on` in `crates/code-core/tests/providers.rs`, `real_gopls_organize_and_format_match_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`. |
| Launch | `gopls` from PATH (`executable`/`args`/`env` under `providers.gopls`); version from `gopls version`. It uses the Go toolchain and module cache of its environment (`go env`). The reader never installs or upgrades it or Go. |
| Workspace root | The nearest `go.work` above the file, else the nearest `go.mod`. |
| Editing (ticket 47) | Verified: problems for the shown version (`undefined: strings`), Organize Imports adding a standard-library import as one Undo step (Ctrl+Alt+O in both adapters), completion of a module dependency's members (`github.com/google/uuid`: module context), parameter information (`ToUpper(s string) string`), gofmt through Format Document (Ctrl+Alt+L in both adapters), and a format answered after an edit refused. |
| Limits | gopls v0.24.0's import guesses follow its own standard-library index: for an unimported `uuid` it adds a bare `"uuid"` path rather than the module's `github.com/google/uuid` with Go 1.24 installed (seen on the command line too). It may download missing modules through GOPROXY when it loads the workspace. Go workspaces with several modules, cgo and build tags untested. |
| Platforms | Linux through the session and GPUI's test platform; macOS unverified. |

## yaml-language-server (YAML)

| | |
| --- | --- |
| Tested version | **yaml-language-server 2.0.0** (npm `yaml-language-server`, Red Hat), Node.js 22.22.0, Linux, 2026-10-26. Fixtures written at test time: `real_yaml_schema_editing`, `real_yaml_downloads_a_remote_schema_only_when_allowed` and `a_missing_yaml_language_server_says_how_to_install_it_and_editing_goes_on` in `crates/code-core/tests/providers.rs`; `real_yaml_schema_assistance_matches_in_both_adapters` and `yaml_enter_and_tab_indent_alike_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`. |
| Launch | `yaml-language-server --stdio` from PATH (`executable`/`args`/`env` under `providers.yaml-language-server`); version from its npm `package.json` (it names itself `unknown` otherwise), read, never run. It needs Node.js 22.13 or newer. Explicit managed installation is available above; upgrades never happen automatically. |
| Workspace root | No project model: the browsing root the file is in, else its folder. Relative paths in `schemas` are resolved from it. |
| Schemas | Only explicit or project-resolved ones: `schemas` in the provider's `settings` (schema path → glob or list of globs, as yaml-language-server's `yaml.schemas`), globally or per project under `projects.<workspace root>`; a top-level `$schema` key or a `# yaml-language-server: $schema=…` modeline in the file, relative to the file. A schema may be JSON or YAML. Without one, only YAML syntax is checked. |
| Privacy | **By default nothing is downloaded.** Lenz turns off the SchemaStore catalog (`schemaStore.enable: false`) and the Kubernetes CRD store (`kubernetesCRDStore.enable: false`), and asks the server to request remote schemas from Lenz (`yaml/registerContentRequest`), which refuses every one: an `http(s)` schema (a modeline, a mapping, the `kubernetes` keyword, a `$ref`) becomes the file's problem "Unable to load schema from '…': Lenz does not download schemas; use a local schema file, or set `remoteSchemas` to true in this provider's settings." `"remoteSchemas": true` in its `settings` lets the server download remote schemas itself; `"schemaStore": { "enable": true }` turns SchemaStore on and allows downloads too (its schemas are remote). Either takes effect on Restart Language Server. Server Details says which: "Schemas: local files only (… schema mappings in providers.json, `$schema` keys and modelines in files); remote schemas and the SchemaStore catalog are not downloaded …" (settings only; no file contents or environment values). A schema path in a file may name any local file the server can read; it is read only as a schema. No YAML content runs a command: the server's only command (Jump to Schema) is not run (Lenz runs server commands only from its allowlist, and does not offer the window capability that enables it). Verified: the server's HTTP proxy (set in its `env`, recorded by the test) hears nothing while a remote modeline is refused, and hears exactly the one GET when `remoteSchemas` is on. |
| Settings answered | Its `yaml` section is its `settings` (Lenz's defaults merged under them, `format.enable: true` among them: with `format` settings that leave out `enable`, 2.0.0 stops formatting); `yaml.format` the same `format`; `http` the user's `settings.http` (a proxy), else nothing; `[yaml]` `{ "editor.tabSize": N }` with N the indent size `.editorconfig` gives a YAML file at the workspace root (else 2), and `editor` `{ "detectIndentation": false }`, unless the settings name those sections: the server indents its suggestions and quick fixes by that size (unanswered, 2.0.0's tab quick fix inserts the text `undefined`). |
| Editing (ticket 53) | Verified, the files on disk unchanged afterwards: problems at their exact ranges, UTF-16 columns after a non-BMP character on the line (`Value is not accepted. Valid values: "fast", "safe".` on an enum value, `Incorrect type. Expected "string".` on a list item, `Missing property "host".` on a nested key; from a project mapping and from a modeline); key completion with the schema's description as documentation (`retries`, "How often to retry."), its snippet `retries: ${1:0}` leaving the `0` selected, one Undo step; an enum's values (`fast`, `safe`, described) replacing the value there; hover with the key's description; the quick fix "Convert Tab to Spaces" using the project's four spaces, one Undo step (offered with "Convert all Tabs to Spaces"); Format Document (the server's bundled Prettier) with `.editorconfig`'s size and the user's `format.singleQuote`, one Undo step. In both adapters: the same problem, Ctrl+Space after a typed `re` listing `retries` with its description (each adapter draws the description beside its list), Enter accepting it, Enter after a new key indenting by the project's size, Ctrl+Alt+L, Ctrl+Z, and Server Details (Find Action) with the same Schemas row. |
| Indentation | YAML indents with spaces only: an EditorConfig or project choice of tabs is overridden ("YAML indents with spaces only" in Editing Preferences), its size kept; YAML's default size is 2. Enter in a YAML draft (both adapters, with or without the server) starts the new line at its line's indentation, under a sequence item's content after `- `, and one step deeper after a key that opens a block (`key:`, a trailing comment aside) or a block scalar (`key: |`, `>-`); each caret its own, one transaction. A pasted line break is literal. Only the caret's own line is read. |
| Limits | Completion indentation follows the root's `.editorconfig` size, not a per-file one. The server's on-type formatting (`\n`) is not asked; Lenz's own Enter rule applies. Code lenses, document links, rename, folding and selection ranges from the server are not used. A project's own Prettier configuration (`.prettierrc`) is not read by its formatter. Custom tags need `customTags` in the settings. Remote schemas, SchemaStore and Kubernetes schemas were exercised only as refused (and one local download through the test's proxy); real SchemaStore schemas are unverified. |
| Platforms | Linux through the session, GPUI's test platform and ratatui's test backend; macOS, Wayland and physical terminals unverified. |

## bash-language-server, ShellCheck and shfmt (Bash, sh)

| | |
| --- | --- |
| Tested versions | **bash-language-server 5.8.1** (npm), Node.js 22, **ShellCheck 0.9.0** and **shfmt 3.8.0** (Ubuntu packages), Linux, 2026-10-26. Fixtures written at test time: `real_shell_editing`, `shellcheck_checks_as_the_identified_shell_and_its_fix_applies_as_one_undo_step`, `the_shell_setting_names_a_scripts_shell_and_zsh_is_left_unchecked` and `a_missing_shell_toolchain_says_what_is_missing_and_editing_goes_on` in `crates/code-core/tests/providers.rs`; `real_shell_assistance_matches_in_both_adapters` and `shellcheck_problems_fixes_and_a_missing_shfmt_match_in_both_adapters` in `crates/code-tui/src/parity_tests.rs` (the non-`real` ones use stand-in tools written by the test). |
| Launch | `bash-language-server start` from PATH (`executable`/`args`/`env` under `providers.bash-language-server`); it names its version when initialized. It serves every file Lenz detects as a shell script (`.sh`, `.bash`, `.zsh`, `.ksh`, `.bashrc`, `.profile`, a shell shebang…). Explicit managed server installation is available above; ShellCheck and shfmt remain separate manual installs. |
| Workspace root | No project model: the browsing root the file is in, else its folder. |
| Which shell | Lenz identifies the shell itself, first match winning: the file's `# shellcheck shell=` directive (before its first command); its shebang (`#!/bin/sh`, `#!/usr/bin/env bash`, `#!/bin/dash`, `#!/usr/bin/env -S bash -e`); a file name or extension that names one shell (`.bash`, `.ksh`, `.zsh`, `.bashrc`, `.bash_profile`, `PKGBUILD`, `.profile` (sh), `.zshrc`; `.sh` names none, since Bash and sh scripts both use it); the `shell` key of the provider's `settings` (an explicit choice, per project under `projects.<workspace root>` or global: `sh`, `bash`, `dash`, `ksh`, `zsh`, also `posix`, `mksh`, `ash`); `shell=` in the nearest `.shellcheckrc` (or `shellcheckrc`) above the file; EditorConfig's `shell_variant`; else Bash. `ash` and BusyBox's sh are checked as dash (ShellCheck 0.9 has no BusyBox dialect), Korn shells as ksh (shfmt's `mksh`). A shebang naming another program (`python3`) is not a shell: neither tool runs. Server Details names the shell and what named it. Unlike ShellCheck run by hand, where a `.shellcheckrc` `shell=` overrides even a shebang, Lenz keeps the shebang (the shell that runs the script) and passes the shell it identified to both tools. The user's own `~/.shellcheckrc` is not read for the shell (ShellCheck still applies its other options when no project file is found). |
| Problems | Built-in Bash-compatible parsing reports structural syntax errors without a server or ShellCheck; zsh and non-shell shebangs are excluded. ShellCheck adds lint diagnostics, run by Lenz (never by the server) on the text shown, on standard input in the script's folder (so the project's `.shellcheckrc` applies its other options), 300 ms after edits pause, within 10 s, in its own process group, off the UI thread, one worker for the window: `--format=json1 --shell=<shell> --external-sources --source-path=<the script's folder>`, then the settings' `shellcheckArguments` without `--shell`/`--format` (`shellcheckExternalSources: false` drops the two source options; an empty `shellcheckPath` turns it off; `shellcheckPath` names another executable). Its columns are characters (a tab is one), placed exactly after characters outside the BMP. Problems describe exactly the version checked, as servers' do. The server's own ShellCheck is turned off: 5.8.1 checks a script without a shebang as Bash whatever its `.shellcheckrc` says, and places ShellCheck's character columns as UTF-16 units, a column short after each character outside the BMP (both seen over its plain protocol). A zsh script is not checked (ShellCheck has no zsh), and the problems list says so. Problems bash-language-server publishes itself (unresolved `source` files, when its `enableSourceErrorDiagnostics` is on) are not shown for shell scripts. |
| Completion | bash-language-server's: the script's variables and functions (and those of files it sources), reserved words, builtins, installed commands and its snippets. An sh or dash script is not offered Bash's own reserved words (`[[`, `function`, `select`, `time`), builtins (`declare`, `typeset`, `shopt`, `source`…; `local` too for sh) or snippets written with Bash syntax (`[[ ]]`, `(( ))`, `declare`, `+=`, array elements), by the shell as Lenz last identified it. |
| Quick fixes | ShellCheck's own fix for a problem at the caret ("Apply fix for SC2086: Double quote to prevent globbing and word splitting."), then "Disable ShellCheck rule SC2086 for the entire file" (a `# shellcheck disable=` line after the shebang), each one Undo step, for the version checked only (Alt+Enter in both adapters). bash-language-server's own quick fixes and formatting are not asked (its capabilities for them are ignored). |
| Format Document | shfmt, run by Lenz on the draft through standard input as the identified shell (`-ln=posix` for sh and dash, `bash`, `mksh` for ksh), with this file's indentation as Lenz resolves it (`-i=N`, or `-i=0` for tabs: EditorConfig, else the project's choice, else four spaces) and each of shfmt's EditorConfig keys (`binary_next_line`, `switch_case_indent`, `function_next_line`, `keep_padding`, `space_redirects`), else the provider's `shfmt` settings (`binaryNextLine`, `caseIndent`, `funcNextLine`, `keepPadding`, `simplifyCode`, `spaceRedirects`; `ignoreEditorconfig` and `additionalArguments` as bash-language-server reads them; `shfmt.path`). Lenz passes them because shfmt ignores EditorConfig once a dialect is given. One Undo step; a format answered for changed text, a failure (Bash arrays formatted as sh: "arrays are a bash/mksh feature") or a missing shfmt changes nothing; format on save uses the same path. Organize Imports is disabled for shell scripts. zsh is not formatted (shfmt 3.8 has no zsh). |
| Editing (ticket 54) | Verified, the files on disk unchanged and no script run (each would leave a file behind): a Bash script and an sh script with the same body get their own shell's problems (Bash: SC2086 on `$name` only; sh also SC3043 `local`, SC2086 `$1`, SC3010 `[[ ]]`) at exact ranges after `😀`; a script without a shebang the sh of its project's `.shellcheckrc` (and, with a stand-in ShellCheck, the per-project `shell` setting over it, and a zsh script left unchecked); completion of the script's variable after `$gree` and of its function after `gre`, each one Undo step, and Bash's `typeset` offered after `ty` in the Bash script but not in the sh one; ShellCheck's fix as one Undo step; Format Document with `.editorconfig`'s three spaces and `switch_case_indent`, one Undo step, as Bash and as sh, and refused for an sh script with a Bash array. In both adapters, with the real tools: the same problem, Ctrl+Space after `$gree` with the same suggestions drawn in both and Enter accepting one, Alt+Enter listing the fixes (drawn by each) and Enter applying one, Ctrl+Alt+L, each Ctrl+Z, and Server Details (Find Action) with the same Server version, Shell, Problems and Format Document rows; with a stand-in ShellCheck and shfmt missing: the problem marked by each, F8 to it, Alt+Enter and Enter, Ctrl+Z, Ctrl+Alt+L saying shfmt is missing, Server Details alike, and typing and Ctrl+S writing the same bytes. |
| Server Details | Rows for the active script: "Shell: sh, from the shebang `#!/bin/sh`." (with the order above); "Problems: ShellCheck 0.9.0 (/usr/bin/shellcheck), checking as sh; …" or "ShellCheck is missing (`shellcheck` is not on PATH)" with how to install it; "Format Document: shfmt 3.8.0 (/usr/bin/shfmt), as `-ln=posix` …" or missing; and that additional lint diagnostics, quick fixes and formatting come from those tools. Versions come from each tool's `--version`, run once per executable. |
| Without the tools | Without bash-language-server, problems, quick fixes and formatting still work when ShellCheck and shfmt are installed; without ShellCheck, built-in structural syntax checks remain available for eligible scripts, while lint diagnostics and quick fixes say the tool is missing; without shfmt Format Document says so; typing, Undo and Save are unaffected. Disable Language Server for This Folder also stops ShellCheck there (Format Document still runs shfmt). |
| Never run | Neither Lenz nor its tools run or source the script: ShellCheck and shfmt read it from standard input, and ShellCheck reads the files it sources (`--external-sources`) as text. bash-language-server parses scripts with its own grammar; it runs Bash's installed completion scripts to complete a command's options and `help`/`man` to document builtins and installed commands (never words that are not one), as it does elsewhere. Its explainshell lookups (which would send words of the script to a web service) stay off unless the settings name an endpoint. |
| Limits | bash-language-server 5.8.1 sometimes answers the first completion after an edit inside an unterminated double-quoted string with nothing (also over its plain protocol); asking again lists the suggestions. It analyses up to 500 shell files of the workspace in the background (`backgroundAnalysisMaxFiles`). It offers no parameter information. zsh scripts get only the server's Bash-grammar assistance. ShellCheck's fixes whose edits overlap are listed as not applicable. Only the nearest `.shellcheckrc` is read for the shell. Bats files, ShellCheck 0.10's BusyBox dialect and shfmt's zsh support (3.10+) are untested. |
| Platforms | Linux through the session, GPUI's test platform and ratatui's test backend; macOS, Wayland and physical terminals unverified. |

## Pyright (Python)

| | |
| --- | --- |
| Tested version | **Pyright 1.1.414** (`pyright --version`), Node.js 26.8.1, CPython 3.14 standard library, Linux, 2026-10-13. Fixture: `real_pyright` in `crates/code-core/tests/providers.rs` (written by the test, pinned in its source). |
| Launch | `pyright-langserver --stdio` from `PATH`, or `executable`/`args`/`env` under `providers.pyright`. It needs Node.js on the `PATH` it runs with (`env.PATH`); the reader never reads shell startup files, so a version manager's shims need an explicit `env.PATH`. Its version is read from the npm package that holds the executable (`package.json`, symlinks resolved); the language server has no version flag and names none when initialized, and the reader never runs the `pyright` command (the PyPI wrapper of that name downloads Node.js and Pyright when it first runs). An unreadable version is shown as unknown and untested; Pyright still starts. Evidence recipe: `READER_PYRIGHT=<node 26.8.1>/bin/pyright-langserver READER_PYRIGHT_PATH=<node 26.8.1>/bin:/usr/bin:/bin` (a non-default mise Node install, set as `executable`/`env.PATH`). |
| Initialization | No options; `workspace/configuration` sections `python[.…]` answer from `providers.pyright.settings` (null without), `pyright` with null. Hierarchical document symbols advertised. |
| Workspace root | The nearest `pyrightconfig.json`, else the nearest `pyproject.toml`/`setup.py`/`setup.cfg`, never above the browsing root; else the browsing root; else the file's folder. A nested project gets its own server; a `pyrightconfig.json` higher up wins over a nearer project file (a project nested inside one is part of it unless its root is set explicitly). |
| Readiness | Pyright reports progress but no readiness status; for 10 s after it initializes its answers are marked possibly incomplete (it answers workspace symbols with nothing until its workspace has loaded). |
| Operations | Definition (functions, classes, methods, standard library through bundled stubs and the interpreter's own sources, several candidates for overloads), usages (complete when asked at the declaration; asked at a use, that use is left out), workspace symbols, file symbols (nested; parameters are listed). |
| Dependencies | File targets: Pyright's bundled typeshed stubs ("Pyright's bundled type stubs"), installed packages (`site-packages`), the standard library. No virtual sources. |
| Python environment (ticket 52) | Lenz reads the settings Pyright reads, at each launch (off the UI thread): `venvPath` and `venv` in the workspace root's `pyrightconfig.json` (comments and trailing commas allowed), else in `[tool.pyright]` of its `pyproject.toml` (a `pyproject.toml` without that section sends Pyright 1.1.414 to the nearest `pyrightconfig.json` above, and Lenz too); `venv` without `venvPath` takes Lenz's `python.venvPath` setting; else `python.pythonPath` in `providers.pyright.settings` (a project's configuration wins over it, as in Pyright). Server Details then shows `Python environment: <folder> (venv "env" with venvPath "." in <file>), Python 3.13.17; installed packages from <site-packages>` (the version from `pyvenv.cfg`, else the interpreter's `--version`, 3 s limit). With nothing configured it names the first `python3` (else `python`) on Pyright's PATH, which Pyright uses, and a project `.venv`/`venv` it does not use. A configured environment that does not exist (or has no `lib/python3.*/site-packages`), a `venv` without any `venvPath`, a missing `pythonPath` or an unreadable configuration file stops Pyright before it starts (status "LSP stopped", the reason and what to fix in the status and Server Details): Pyright 1.1.414 would otherwise fall back to the PATH interpreter's packages and say so only in its log. Typing, Undo, Save and formatting go on. Lenz never creates environments or installs packages. |
| Editing (ticket 52) | Verified with Pyright 1.1.414 (the PyPI `pyright` 1.1.414 package's bundled npm package, its `langserver.index.js` linked as `pyright-langserver` the way npm links it, so its version is read) on Node.js 22.22.0, CPython 3.13.17 virtual environments made with `python3 -m venv` and a dependency installed by the environment's own pip 26.2.2 (offline, from a wheel the test writes), ruff 0.16.9 and black 26.5.1, Linux, 2026-10-26: with `pyrightconfig.json` naming the project's environment while another one is first on Pyright's PATH, Server Details names the configured one, the dependency's members complete (one Undo step), its parameter information shows `(radius: float) -> float` with the active parameter, its definition opens in that environment's `site-packages` ("an installed Python package"), and the other environment's package is reported unresolved (`Import "other_dep" could not be resolved`); without configuration the PATH environment is used and named, and the project's unconfigured `.venv` is named as unused; a missing configured environment stops Pyright and editing goes on. Pyright offers no quick fixes for these problems (Show Quick Fixes says none are offered) and no organize imports or formatting. **Format Document and Organize Imports run the project's formatter instead**: ruff when `ruff.toml`, `.ruff.toml` or `[tool.ruff]` in `pyproject.toml` configures it (`ruff format --no-cache --stdin-filename <file> -`; Organize Imports `ruff check --no-cache --select I --fix --exit-zero --stdin-filename <file> -`, its import-sorting rules only), else black for `[tool.black]` (`black --quiet --stdin-filename <file> -`; it does not organize imports), found in the configured environment's `bin` (else, with none configured, the project's `.venv`/`venv`; with a missing one, only PATH), else on PATH. They read the draft on stdin, find the project's configuration from the file name, and the answer is one Undo step when the text is still the version given (a format answered after an edit is refused), with format on save writing the formatted draft. Format Document works while Pyright is missing or stopped. Nothing is written to the project (`--no-cache`: no `.ruff_cache`). Fixtures: `real_pyright_configured_environment_editing`, `real_pyright_missing_environment_is_not_replaced`, `real_pyright_without_configuration_says_which_environment_it_uses`, `real_black_formats_python_without_a_language_server` (`crates/code-core/tests/providers.rs`) and `real_pyright_environment_and_ruff_match_in_both_adapters` (`crates/code-tui/src/parity_tests.rs`: Server Details through Find Action, typed completion with Down/Enter, Ctrl+Z, Ctrl+Alt+L, Ctrl+Alt+O in both). |
| Limits | Static analysis only (no dynamic attributes or monkey patching); unresolved imports are not followed; files under hidden folders are outside its workspace by default (no workspace symbols for them). The environment is checked when Pyright starts: a configuration or environment changed while it runs is reloaded by Pyright itself (it may then fall back silently), and Lenz checks it again on **Restart Language Server**. Conda environments, `python.defaultInterpreterPath`, `${workspaceFolder}`-style variables in settings, execution environments (`executionEnvironments`), Windows and macOS are untested. ruff's lint problems and fixes are not shown (only its formatter and import sorting run); other formatters (autopep8, yapf, isort alone) are not run. The PyPI `pyright` wrapper's own `pyright-langserver` (a Python script) shows its version as unknown. |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 2.1 ms, p95 3.2 ms. Usages listed: median 3.2 ms, p95 3.2 ms. Server memory 171 MiB, no helpers. |
| Platforms | Linux verified through the session and through GPUI's test platform (no private display: Xvfb is absent); macOS unverified. |

## typescript-language-server (TypeScript, TSX, JavaScript)

| | |
| --- | --- |
| Tested version | **typescript-language-server 6.0.1** with **TypeScript 6.0.3** (`tsserver.js` from the VS Code install, named by the user's `tsserver.path` setting), Node.js 26.10.0, Linux, 2026-10-13. Fixture: `real_typescript` in `crates/code-core/tests/providers.rs`, pinned in its source, with parts per language: TypeScript (`src/geometry.ts`, `src/app.ts` with CRLF and a non-BMP character, `src/extra.ts`), TSX (`src/view.tsx`), JavaScript (`src/util.js`, `src/main.js`), a nested package (`packages/ui`) and an `@types` package. |
| Launch | `typescript-language-server --stdio` from `PATH`, or `executable`/`args`/`env` under `providers.typescript-language-server`; needs Node.js on its `PATH`. Version from `--version`. It uses the project's `node_modules/typescript`, else the user's `settings.tsserver.path` (passed in `initializationOptions`); TypeScript 7 (the native compiler) has no tsserver, so with only that installed it fails to initialize and the reader shows its message. The reader never chooses a TypeScript for the user. |
| Languages | TypeScript (`typescript`), TSX (`typescriptreact`), JavaScript (`javascript`, `.jsx` as `javascriptreact`): one owner; TSX never goes to the HTML server. |
| Workspace root | The outermost `tsconfig.json`/`jsconfig.json`/`package.json` under the browsing root, else the browsing root (without one: the nearest, else the file's folder). Nested configurations share that server (tsserver resolves each file's project itself). |
| Readiness | No readiness status: for 10 s after it initializes, answers are marked possibly incomplete, and a single definition jumped to then carries a notice. Early definitions can stop at the import binding (seen at 0.3 s) until tsserver has loaded the project. |
| Operations | Definition: TypeScript → TypeScript, TypeScript → JavaScript, TSX → TypeScript, JavaScript → JavaScript, into TypeScript's library declarations and an `@types` package. Usages from the declaration: TypeScript (TS, TSX, nested package) and JavaScript (JS and TS users), imports included. Workspace symbols (rows show the declaration's line); file symbols (nested; constructors and parameter properties listed). A replaced question and a stale result (file changed after listing) behave as for every provider. |
| Dependencies | File targets: TypeScript's library declarations (`lib.*.d.ts`), `@types` packages, other `node_modules`. No virtual sources. |
| Limits | Navigation follows TypeScript's project model; plain JavaScript is resolved by inference; a TypeScript with tsserver is required (where only TypeScript 7 is found, typescript-go serves instead, below); TypeScript 6 includes no `@types` package unless `compilerOptions.types` names it. Without a browsing root, a file's server is rooted at its nearest project file (so one monorepo opened file by file can get more than one server). |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 2.1 ms, p95 3.2 ms. Usages listed: median 2.1–3.2 ms, p95 3.2 ms. Memory: server 76–78 MiB, tsserver (its helper) 656–675 MiB. |
| Platforms | Linux verified through the session and GPUI's test platform (no Xvfb); macOS unverified. |
| Editing (ticket 48) | Verified on typescript-language-server 6.0.1 (npm) with TypeScript 6.0.2's tsserver, Node.js 22.22.0, Linux, 2026-10-26, with `real_typescript_editing` in `crates/code-core/tests/providers.rs` and `real_typescript_fix_and_format_match_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`: completing another module's export with its import after a non-BMP character on the line (UTF-16 ranges) as one Undo step, problems for the shown version, the "Update import from …" fix, parameter information, Format Document, Organize Imports (it sorts and combines; it never removes unused imports), JSDoc-typed members in a JavaScript file and a component's props in TSX. Its fixes arrive as `_typescript.applyCodeActionCommand` carrying tsserver's own action: Lenz never runs it, and applies the text changes it carries only when they change this file alone and ask for no further command (an `@types` install, say); "Move to a new file" and other refactorings that need the command stay refused. Answers are empty until tsserver has loaded the project. |

## typescript-go (TypeScript 7: TypeScript, TSX, JavaScript)

| | |
| --- | --- |
| Tested version | **typescript-go 7.0.2** (`tsc --lsp --stdio` of the npm `typescript` 7.0.2, global under Node.js 26.10.0), Linux, 2026-10-17. Fixtures: `real_typescript_go` and `real_typescript_7_installation_chooses_typescript_go` in `crates/code-core/tests/providers.rs` (the TypeScript fixture above); the choice and launch also with a fake server and in unit tests. |
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
| Tested version | **JDT Language Server 1.61.0-SNAPSHOT** (its `initialize` answer; Arch package `jdtls 1.61.0-1`), its Python launcher on Python 3.14, Java from `JAVA_HOME` = Temurin 21.0.2 (sdkman), Linux, 2026-10-13. Fixture: `real_jdtls` in `crates/code-core/tests/providers.rs` (a plain source tree with Unicode identifiers and a CRLF file, pinned in its source). |
| Launch | `jdtls --jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false -data <reader cache>/jdtls/<project>-<hash>` (`$XDG_CACHE_HOME/lenz-code`): a workspace-data folder per project root and launch, so projects (even with the same folder name) and launch configurations never share one; jdtls's own default is keyed by the folder's name alone. `executable`/`args`/`env` (PATH, JAVA_HOME) under `providers.jdtls`, where `args` add to the fixed metadata property (never replace it; it is not part of the data folder's identity, so folders made before it keep being used); its launcher needs Python 3 and Java 21+. Version from its `initialize` answer (the launcher has no version flag). |
| Initialization | `extendedClientCapabilities.classFileContentsSupport: true`, `settings.java.autobuild.enabled: false` (a reader never builds) and `settings.java.signatureHelp.enabled: true` with descriptions (JDT LS answers parameter information only when a client turns it on), with `providers.jdtls.settings` merged over them object by object (a user's `settings.java…` keeps the others); `workspace/configuration` for `java[.…]` from those settings. |
| Workspace root | The outermost `pom.xml`/`build.gradle[.kts]`/`settings.gradle[.kts]`/`.project` under the browsing root (it imports nested modules itself), else the browsing root; without one, the nearest, followed up through parent Maven/Gradle builds. |
| Readiness | `language/status`: busy from `Starting` until `ServiceReady` (1.4 s on the fixture); `Error` messages are shown; build/import progress is shown. |
| Operations | Definition (within the project; JDK classes as retrieved source), usages (from the declaration), workspace symbols (types: a class name finds it; a method name, `twice`, found nothing; rows show the declaration's line), file symbols (nested; the package declaration is listed). |
| Editing (ticket 44) | Verified on JDT LS 1.61.0 (Eclipse milestone tarball), OpenJDK 21.0.12, Maven 3.9.11 and Gradle 8.14.3, Linux, 2026-10-25, with `real_jdtls_maven_editing` and `real_jdtls_gradle_editing` in `crates/code-core/tests/providers.rs` and `real_jdtls_import_fix_and_format_match_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`. A Maven and a Gradle project depending on Gson: problems for the shown version (`Gson cannot be resolved to a type`), the dependency's "Import 'Gson' (com.google.gson)" quick fix applied as one Undo step (Alt+Enter, selected by keys, in both adapters), completion of `ArrayList` with its `import java.util.ArrayList;` as one Undo step, parameter information across Gson's eight `toJson` overloads, Format Document as one Undo step (Ctrl+Alt+L in both adapters), a format answered after an edit refused, and Organize Imports removing an unused import. Its "Create class/enum/interface/record" fixes write a new file and are listed as not applied. JDT LS attaches `java.completion.onDidSelect` to every suggestion; Lenz does not run it (it only ranks later suggestions), so recently chosen suggestions are not ranked higher. |
| Virtual sources | `jdt://` targets (JDK and library classes) are retrieved through `java/classFileContents`, from the server session that named them, into a kept read-only tab ("PrintStream.java (Eclipse JDT LS)") with the URI in the status bar; never written, watched, reloaded or sent back to a server; after that session ends the tab says so; once closed it is not held. Several virtual targets only: the first is retrieved and the count said; virtual targets beside file targets are counted, not listed. Peek on a virtual target opens the retrieved tab (said so): there is no file to preview. |
| Side effects | A folder without a build file: nothing written into the project (verified). A Maven project (verified with the Kotlin fixture's `pom.xml`, `real_kotlin_with_jdtls`): imported by JDT LS through m2e, which resolves plugins and dependencies through `~/.m2` (and the network for anything missing); by default it also wrote `.project`, `.classpath`, `.settings/` and built `target/classes` into the project, so the reader launches it with `--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false` (Eclipse metadata kept in its workspace-data folder; a JVM property because it is read at startup, as VS Code's Java extension passes it) and `java.autobuild.enabled: false` in its settings: nothing written into the project (verified). A Gradle project imported with the installed Gradle (`settings.java.import.gradle.home`) took 28.6 s to ready the first time; files written into the project by the Gradle import were not checked. Workspace-data folders stay in the reader's cache (hundreds of MiB per imported project) and are not cleaned up; two reader windows on the same project share one folder, which Eclipse locks (the second server fails, with its message). |
| Limits | Kotlin files are not part of its index (its usages say they leave out Kotlin callers when the project has Kotlin files); definitions from inside retrieved source are not available; quick fixes and refactorings that create or edit other files are not applied; Organize Imports with ambiguous types (JDT LS then asks the client to choose) is untested. |
| Measured (release, fixture, 100 samples after 5) | Warm definition to target shown: median 58.9 ms, p95 61.2 ms (the server's own time; other providers answer in 2–3 ms). Usages listed: median 5.3 ms, p95 7.4 ms. Memory: 2.1–2.2 GiB (`-Xms1G` and its heap; above the reader's 2 GiB notice on its own). |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click into retrieved JDK source; no Xvfb); macOS unverified. |

## VS Code HTML and CSS servers (HTML, CSS)

| | |
| --- | --- |
| Tested version | The servers built into **VS Code 1.141.0** (Arch `visual-studio-code-bin 1.141.0-1`: `html-language-features` and `css-language-features` 10.0.0), run with Node.js 26.10.0, Linux, 2026-10-13. Fixtures: `real_html` and `real_css` in `crates/code-core/tests/providers.rs` (a page with an inline `<style>` and `<script>`, a non-BMP character and non-ASCII names; a CRLF stylesheet with a non-ASCII custom property), pinned in their source; `real_html_and_css_through_the_window` in the GUI tests. Also **vscode-langservers-extracted 4.10.0** (npm, its `vscode-html-language-server` and `vscode-css-language-server`) with Node.js 22.22.0, Linux, 2026-10-26: `real_html`, `real_css`, `real_html_editing` (`crates/code-core/tests/providers.rs`) and `real_html_assistance_matches_in_both_adapters` (`crates/code-tui/src/parity_tests.rs`). Both count as tested (`1.141.0 or 4.10.0`). |
| Launch | `vscode-html-language-server --stdio` / `vscode-css-language-server --stdio` from `PATH` (npm `vscode-langservers-extracted`; version from its `package.json`). When those are not on `PATH` and the settings name no `executable`, the server an installed VS Code ships (`/usr/share/code`, `/opt/visual-studio-code`, `/usr/lib/code`, the snap, the macOS app) runs as `node <script> --stdio`, with `node` from the launch's `PATH`; its version is the VS Code version (`package.json` above the extensions folder), read, never run. Without Node.js it is missing, said so ("needs Node.js"). Other installs (Flatpak, `~/Applications` on macOS, a tarball) are not searched: set `executable` (`node`) and `args` (the script, `--stdio`). The choice between PATH and VS Code is made when the server starts: a later install or VS Code update is picked up by Restart Language Server. The script runs with the same trust as launching that VS Code (a system install location; no ownership check). `executable`/`args`/`env` under `providers.vscode-html` / `providers.vscode-css`. |
| Initialization | HTML: `provideFormatter: true`, `embeddedLanguages: { css: true, javascript: true }`; CSS: `provideFormatter: false`. The user's `settings` are merged over them and answered for `html[.…]` / `css[.…]`. The HTML server also reads the `css`, `javascript` and `js/ts` sections for its `<style>` and `<script>` blocks: each is answered with the user's settings under that key (`settings.css.lint`, `settings.javascript.format`, …), else `{}` for `css` and `javascript` and nothing for `js/ts`. With a missing `css` section (before 2026-10-26) the server stopped validating every page (`Cannot read properties of null (reading 'validProperties')`), and with an empty `js/ts` it stops validating scripts, so those defaults are deliberate. |
| Languages | HTML (`html`), CSS (`css`). TSX, JSX and JavaScript files stay with the TypeScript provider. SCSS/Less are not detected as CSS by the shared highlighter and get no server. |
| Workspace root | No project model: the browsing root the file is in, else its folder. One server per browsing root and language. |
| Operations | HTML editing: below. CSS: definition and usages of custom properties and `@keyframes` within the same stylesheet (Unicode and CRLF positions verified); file symbols (rules, `:root`, keyframes). HTML: definition and usages inside the page's own `<script>` (functions, variables) and `<style>` (custom properties); file symbols (its elements and style rules). Both include the declaration in usages (they ignore `includeDeclaration: false`). |
| Not provided | A class or id in HTML is not followed to a stylesheet, nor a selector to markup, templates, frameworks or other files: the definition says it found none (no text match stands in). No workspace symbols: the command is disabled with "The language servers running for this folder do not provide workspace symbols; find declarations instead" (not an empty result); syntactic declarations and text search stay available. |
| Dependencies | `node_modules` files show "an npm dependency"; no virtual sources. |
| Side effects | Nothing written into the project (verified: the fixture's files are unchanged afterwards). HTML formatting runs only when asked (Format Document, or Save with the project's format on save) and edits the draft; CSS formatting is off. |
| Editing HTML (ticket 51) | Verified on vscode-langservers-extracted 4.10.0 with `real_html_editing` and the two-adapter replay above, on a page each test writes, the files on disk unchanged afterwards. Problems of the page's `<style>` (`Unknown property: 'colr'`) and `<script>` (`Expression expected.`) at their exact source ranges after a non-BMP character on the line (UTF-16 columns), with the user's `css.lint` settings applied (`emptyRules: ignore`). Completion of a tag name after `𝄞` (the server's range replaces just the typed prefix), of an attribute (`title="$1"`: the caret between the quotes), of a closing tag after `</` (`/li` replaces the `/` with `/li>`), of a CSS property in `<style>` (`color: $0;`: the caret before the `;`, then its values suggested at once, as the server asks with `editor.action.triggerSuggest`) and of the page's JavaScript function in `<script>`, with parameter information (`greet(name: any, punct: any): string`, second parameter active). Closing tags as VS Code closes them: typing `>` after `<li` or `</` after an unclosed element asks `html/autoInsert` (`autoClose`) and inserts its answer (`$0</li>`, `section>`) as its own Undo step, the typed character kept, only while the text and the one caret are those it was typed at (an answer after more typing is dropped; several carets ask nothing); `autoClosingTags: false` in the settings turns it off. Format Document with the file's indentation (three spaces from the fixture's `.editorconfig`) and the user's `format` settings (`indentInnerHtml`, `extraLiners`), the `<style>` and `<script>` blocks formatted as CSS and JavaScript, one Undo step. In both adapters: the problem shown, typed `<li>` closed with the caret inside, `x</` closing it again after Ctrl+Z, a tag name chosen with Down and Enter from the suggestions typing opened (drawn in both), Ctrl+Alt+L, Ctrl+Z; Alt+Enter says "Quick fixes: The language server does not offer quick fixes" and Server Details (Find Action) lists closing tags among what is offered and the embedded languages in the provider limits, the same rows in both. |
| Editing limits | No quick fixes or organize imports (the server offers neither); `autoQuote` (`=` adding quotes) is not asked: the editor's own quote pairing applies. The server's `editor.action.triggerParameterHints` hint is ignored. Only CSS and JavaScript are embedded: `<script lang="ts">` is read as JavaScript, template types (`text/x-template`) get nothing, and frameworks are unknown. With vscode-langservers-extracted 4.10.0 the page's JavaScript has no standard library or DOM types (the package looks for TypeScript's `lib.*.d.ts` one folder too high and logs `Unable to load library`), so `document.` and built-in members are not suggested and parameter types are mostly `any`; VS Code's built-in server ships them (its editing is unverified here: VS Code was not installed). With `format.endWithNewline` off (the server's default) Format Document drops the file's final newline; turning it on also ends each `<style>` block with an empty line. Unfinished markup is formatted as the server reads it (an unclosed element nests what follows). CSS files have no formatting or editing verification (ticket 51 covers HTML). |
| Measured (release, fixture, 100 samples after 5) | Ready 0.1 s (CSS), 0.2 s (HTML). Warm definition to target shown: median 2.1 ms, p95 2.1 ms (both): the test harness's polling floor (about 2 ms, as for typescript-language-server), so the servers answer at least that fast. Memory: CSS 80–88 MiB, HTML 242–243 MiB (it embeds TypeScript's language service for scripts). Raw samples: `READER_PERF` lines of `real_css` / `real_html`. |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click, Shift+F12, the F10 reason, an external edit, a workspace root set for the page, Back; no Xvfb); macOS unverified (the app path is listed, not exercised). |

## Ruby LSP (Ruby)

| | |
| --- | --- |
| Tested version | **ruby-lsp 0.26.11** (`gem install --user-install ruby-lsp`), Ruby 3.4.10 (Arch), RubyGems 4.0.15, Bundler 4.0.15, Linux, 2026-10-13. Fixture: `real_ruby` in `crates/code-core/tests/providers.rs` (a CRLF file with a non-BMP character, non-ASCII class and method names, a nested bundle, a bundle locked offline to the installed `thor` 1.4.0), pinned in its source; `real_ruby_conflicting_addon_versions` (two installed versions of an add-on gem that activate conflicting versions of another gem); `real_ruby_through_the_window` in the GUI tests. |
| Launch recipe | `ruby-lsp` from PATH (or `executable`/`env` under `providers.ruby-lsp`) inside a bundle Bundler sets up (`RUBYOPT=-rbundler/setup`, as editors' `bundle exec`): the project's own when its lock lists `ruby-lsp`; otherwise one the reader composes in its cache (`$XDG_CACHE_HOME/lenz-code/ruby-lsp/bundles/<project>-<hash>/`): a `Gemfile` that evaluates the project's (`eval_gemfile`) and adds `ruby-lsp` (and `ruby-lsp-rails` for a Rails app when it is installed), with a copy of the project's lock (relative path sources made absolute), resolved by `bundle lock --local` (the `bundle` beside `ruby-lsp`, else on PATH) from installed gems only and redone only when the Gemfile it writes, the project's lock or `PATH`/`GEM_HOME`/`GEM_PATH` change. The project's `.bundle` settings apply (`BUNDLE_APP_CONFIG`; a relative `BUNDLE_PATH` from the project). A failed lock stops the launch with Bundler's message. Outside a bundle (0.0.3), Ruby LSP loads the add-ons of every installed version of every gem: with an RVM gem set holding two RuboCop versions, `initialized` failed with `Gem::ConflictError` and indexing stayed at 0%. Its default launch (no `BUNDLE_GEMFILE`) is not compliant: it writes `.ruby-lsp/` (a composed Gemfile, lockfile and markers) into the project, runs `bundle install` (network) and every four hours `bundle update ruby-lsp debug prism` (an upgrade of the server); the reader never uses it. A user `env.BUNDLE_GEMFILE` or `env.RUBYOPT` wins. Version from `ruby-lsp --version` (it exits before any bundle work). |
| Initialization | `formatter: "none"`, `linters: []` (no RuboCop: it would load the project's configuration and plugins), and the [tooling-gem exclusion preset](#project-language-server-settings); the user's global/project `settings` are merged over them. Positions: UTF-16 only (0.26.11 answers index positions in characters even with UTF-8 negotiated: a non-ASCII name in a file it does not hold open ended a byte short); characters outside the BMP in such names would still be off. |
| Workspace root | The nearest folder with a `Gemfile`/`gems.rb` under the browsing root (each bundle is its own project: engines, nested apps), else the browsing root; without one, the nearest such folder, else the file's. Override per browsing root (verified through the command menu's modal); an overridden root inside a bundle keeps that bundle's Gemfile (the nearest at or above the root). |
| Readiness | No readiness status: for 10 s after it initializes, answers are marked possibly incomplete; on the fixture it indexed within 0.2–0.3 s, RBS core signatures and gems shortly after. |
| Operations | Definition: classes and modules (constants), constant assignments (the whole assignment is selected), methods (by name), across files; into RBS core signatures (`File` → `file.rbs`) and into installed gems (`Thor` is reopened in many of its files: candidates). Usages (from a use or the declaration; the declaration is included: it ignores `includeDeclaration: false`): a constant's (`AREA_UNIT`: its use and assignment) separately from a method's (`área`: every call by name, any receiver, and its `def`). Workspace symbols: fuzzy over qualified names (`Shapes::Círculo` finds it; a short name inside a long namespace can fall below its threshold); file symbols (nested). Never text matches. |
| Dependencies | Installed gems ("an installed gem", recognized by their installation folders: user and system gem paths, version managers, `vendor/bundle`; a project's own `gems/` folder is not one), RBS core signatures ("RBS core signatures"), the standard library ("the Ruby standard library"), as files; no virtual sources. Gems are indexed from the bundle, at their locked versions. Every locked gem must be installed: otherwise the reader's `bundle lock --local` (or Bundler's setup) fails and the launch stops with Bundler's message; run `bundle install` in the project (the reader never installs gems). Lenz 0.0.3 and earlier skipped uninstalled gems instead. |
| Side effects | Bundler evaluates the project's Gemfile, and Ruby LSP loads the add-ons of gems in the bundle and in the project (`**/ruby_lsp/**/addon.rb`): project code runs, as in any editor running Ruby LSP; the Rails add-on loads the application. It writes nothing into the project (verified on the fixtures), with one exception: a `.ruby-lsp/bundle_env` left by an editor's launch is read and deleted at startup. The reader writes only its composed bundle in its cache; nothing is installed or downloaded. |
| Limits | Static index: dynamic definitions, `method_missing` and metaprogramming are not followed; method usages are by name; external edits reach it through the reader's changed-file reports (verified: a moved class is found at its new line). |
| Measured (release, fixture, 100 samples after 5) | Ready 0.2–0.3 s. Warm definition to target shown: median 2.1 ms, p95 3.2 ms (the harness's polling floor is about 2 ms). Memory: 109–117 MiB (three runs; raw samples in the code-reader build evidence, `real-ruby-3x.txt`). |
| Platforms | Linux verified through the session and GPUI's test platform (Ctrl-click, Back, Shift+F12, the root modal; no Xvfb); macOS unverified. |
| Editing (ticket 49) | Verified on Ruby LSP 0.26.11 and RuboCop 1.92.1 (installed for this ticket with `gem install`), Ruby 3.3.6, Linux, 2026-10-26, with `real_ruby_editing` and `real_ruby_without_a_gemfile` in `crates/code-core/tests/providers.rs` and `real_ruby_lsp_fix_and_format_match_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`. A bundled project with RuboCop in its Gemfile: RuboCop's problems for the shown version, its "Autocorrect Lint/UselessAssignment" fix and its formatting through Format Document, each one Undo step, in both adapters; completion of the project's constant `Shapes::Círculo` on a line after a non-BMP character (UTF-16); parameter information (`new()`: Ruby LSP does not show `initialize`'s parameters). Without a Gemfile it starts in a bundle of its own and completes the project's constants, writing nothing into the folder. Lenz now starts it with `formatter: auto` (and its default linters): RuboCop is used when the project's bundle has it, and loads the project's `.rubocop.yml`. Limits: no method completion on a local variable whose type it does not infer (`c.` after `c = Shapes::Círculo.new(2)`); with UTF-16 its parameter information finds nothing on a line that holds an astral character, while UTF-8 would misplace definitions from its index, so Lenz keeps UTF-16. |
| Rails (ticket 50) | Verified on Ruby LSP 0.26.11 with ruby-lsp-rails 0.4.8, Rails (railties) 8.1.4, sqlite3 2.10.1, RuboCop 1.92.1 and rubocop-rails 2.38.0 (installed with `gem install`), Ruby 3.3.6, Linux, 2026-10-26, on a Rails application each test creates (a model with `has_many` and `validates`, a controller with `before_action`, route helpers and `Time.now`, a helper, `resources :posts`, its schema loaded into a SQLite development database): `real_rails_editing` and `real_rails_that_cannot_boot` in `crates/code-core/tests/providers.rs`, `real_rails_assistance_matches_in_both_adapters` in `crates/code-tui/src/parity_tests.rs`. Lenz adds the add-on to the bundle it composes for an app that locks `railties`; the add-on boots the application (`bundle exec rails runner` from the Rails root) for its context. From the booted application: Quick Documentation of a model lists its columns (`title: string - not null`); `Post.where(ti` suggests `title` ("Filter Post records by title") and inserts `title: `, also on a line typed into the unsaved draft (`Comment.where(te` → `text`), one Undo step; Go to Definition of a route helper (`posts_path`, `post_path(post)` in a helper) opens `config/routes.rb` at its route, of `has_many :comments` the `Comment` model. From the add-on's static analysis: a `before_action :set_post` symbol opens the method; association methods (`comments`, `comments=`) are suggested in the model; `has_many`/`validates` calls are file symbols. Ruby LSP: parameter information from the bundle's Active Record (`find(*ids)`). rubocop-rails' problems (`Rails/TimeZone`) and its "Autocorrect Rails/TimeZone" fix, and RuboCop formatting with the project's `.rubocop.yml`, each one Undo step, in both adapters. An application that cannot boot: the add-on's error is shown in Language Server Status ("Ruby LSP Rails failed to initialize server: …" with the application's own message) and its runtime context is missing, while RuboCop's problems, formatting and constant completion keep working. Limits: suggestions show the add-on's `labelDetails` (file, filter text) as their detail; file symbols show the source text of names (`comments`, not `has_many :comments`); parameter information needs the caret outside a call nested in the parentheses (`find(params[:id])` on `params` answers nothing); answers that need the gem index (parameter information) may be empty for the first seconds of indexing, right after an edit Ruby LSP with the add-on sometimes answers a `where` completion with nothing (reproduced with Ruby LSP alone, outside Lenz; asking again with Ctrl+Space answers); code lenses (route lenses on actions, running tests), the pending-migrations prompt and its **Run Migrations** action are not offered (Lenz does not request code lenses or declare message-action properties); route changes need **Restart Language Server** (the add-on's own limit); the booted application runs project code and writes what it writes when it boots (verified: only SQLite's `-shm`/`-wal` files beside the open development database); ERB views' Rails context (ticket 58; the template route itself: [Templates](#templates-erb-and-haml-ruby-lsp-vs-code-html)), native macOS, Wayland and physical terminals unverified. |

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
`~/.config/lenz-code/providers.json` (or `$XDG_CONFIG_HOME/lenz-code/providers.json`). With rbenv, for example,
the executable can be `/Users/your-name/.rbenv/shims/ruby-lsp`;
[Ruby LSP's editor instructions](https://shopify.github.io/ruby-lsp/editors.html) explain version-manager setup.

For Rails-specific assistance, install `ruby-lsp-rails` with the same Ruby (tested: 0.4.8). Lenz adds it to the
bundle it composes for a Rails app (one whose lock lists `railties`) only when it is already installed; it is not
installed automatically as in the standard Ruby LSP launch, and without it Ruby editing works as for any bundle.
The [Rails add-on](https://shopify.github.io/ruby-lsp/rails-add-on) boots the application from its root
(`bin/rails` must exist) for runtime introspection: model columns in Quick Documentation and as `where`
suggestions, route-helper and association targets. That needs what the application needs to boot (its database
set up, for example `bin/rails db:prepare`); when it cannot boot, Language Server Status shows the add-on's error
and the rest of Ruby editing keeps working. Add `rubocop-rails` to the Gemfile and `.rubocop.yml` for Rails
problems and fixes. Code lenses, the pending-migrations prompt and the add-on's VS Code-specific controls are not
Lenz features; after changing routes, choose **Restart Language Server**. See the Rails row above for what was
verified.

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

## Templates: ERB and Haml (Ruby LSP, VS Code HTML)

Ticket 57: the routing seam templates are served through. The table records ticket 57; [ERB editing](#erb-editing)
(ticket 58) supersedes its ERB limits (Rails view context, formatting) and [Haml editing](#haml-editing) its Haml ones.

| | |
| --- | --- |
| Tested versions | ruby-lsp 0.26.11 with RuboCop 1.92.1 in the project's bundle, Ruby 3.3.6; vscode-langservers-extracted 4.10.0 (HTML server) on Node.js 22; Action View / railties 8.1.4 (Erubi 1.13.1) and Haml 7.5.1 (installed for this ticket into a private gem directory, with Temple 0.10.7, Tilt 2.9.0, Thor 1.5.0, Prism 2.0.0) for the engine cross-checks; Linux, 2026-10-26. Tests: `real_erb_template_editing`, `real_haml_template_editing` (`crates/code-core/tests/providers.rs`), `real_erb_regions_are_the_ruby_rails_compiles`, `real_haml_regions_are_the_ruby_haml_compiles` (`crates/code-core/tests/templates.rs`, the latter with `EDITOR_HAML_GEM_HOME`), `real_erb_template_assistance_matches_in_both_adapters` (`crates/code-tui/src/parity_tests.rs`); fake-server tests in `crates/code-core/tests/semantics.rs` (`an_erb_templates_ruby_…`, `an_html_erb_templates_markup_…`, `template_edits_touching_scaffolding_…`, `a_haml_templates_ruby_…`) and `template_assistance_routes_through_its_projections_alike_in_both_adapters` (both adapters). |
| Files | `*.erb` (`.html.erb`, `.text.erb`, `.js.erb`…), `*.rhtml`, `*.haml` (`.html.haml`), by the last extension. No syntax highlighting yet. |
| Routing | The template is never sent as it is. Its **Ruby** goes to Ruby LSP as a projection: each ERB code/output tag's code (Rails' Erubi reading: `<%-`, `-%>`, `=%>`, `<%==` handled; `<%#` comments and `<%%` literals are not Ruby), or each mapped Haml script and `#{…}` interpolation, copied verbatim, a newline after each, sent under the template's own URI and version as `languageId: ruby` (Ruby LSP's own ERB mode, with its unsafe replacement ranges, is not used). An HTML ERB template's **markup** (`.html.erb`, `.htm.erb`, `.rhtml`) goes to the HTML server under the same URI as `languageId: html`: the same bytes with every tag, delimiters included, replaced by `_` (line ends kept), so a `<script>` or an attribute holding a tag still parses. The caret decides which server a question goes to; Format Document goes to the Ruby. A caret on a tag's delimiters or in a comment, in Haml markup or in a non-HTML template's text asks nothing and says why. Positions are converted in each server's negotiated encoding against its projection and mapped back byte-exactly. |
| Edits | A server's edit (completion with its extra edits and snippets, quick fixes, formatting, server-command edits, closing tags) applies only when every part lies inside one mapped region and the touched tag, markup stretch or Haml line still reads as the same construct afterwards (no new `%>`, `<%`, comment or trim marker, no other Haml line kind); otherwise the whole transaction is refused with the reason ("it reaches outside the template's embedded Ruby", "it would change the template's own tags, delimiters or lines"), as are several carets when any of them falls outside, another file's edits and answers for an older version. Each applied change is one Undo step; Save writes the draft as edited. |
| Verified (real servers) | ERB: Ruby's syntax problems at the template's own `]` after CRLF, non-BMP text and multiline Ruby; `Greeter.gre` completed to `greet` inside an output tag in an attribute; parameter information `greet(name, excited: <default>)` inside calls (keyword parameter active; inside a string on a line holding `𝄞`, UTF-16); a tag name (`span`) completed in the markup after `𝄞`; a typed `<b>` closed by the HTML server; Format Document "nothing to change"; through both adapters (typed completion with Enter, Ctrl+Shift+Space / the terminal's Parameter Info, Ctrl+Alt+L, Ctrl+Z). Haml: syntax problems at `]`, completion in a `%li=` script, parameter information inside `#{…}`. Every mapped region of the fixtures is Ruby that Action View's ERB handler and Haml 7.5.1 compile verbatim. Files on disk unchanged. |
| Not shown | RuboCop and Standard Ruby problems about the projection (Ruby LSP 0.26.11 runs neither on a `.erb`/`.haml` URI anyway), and Ruby's "possibly useless use … in void context" about an output (`<%= x %>` renders `x`). Semantic highlighting and inlay hints are not requested for templates. |
| Limits | No Rails view context: helpers, controller instance variables and partial locals are unknown to Ruby LSP here (ticket 58). Ruby LSP does not complete methods without a receiver (`<%= gre %>` with a top-level `def greet`: nothing, as in a `.rb` file); parameter information right after a freshly typed `(` answers nothing (ticket 04's private signature preload is not shipped). Format Document formats nothing (RuboCop skips templates; erb-formatter, htmlbeautifier or haml-lint are not run; ticket 58/59). Haml markup gets no assistance (it is not HTML; ticket 59). A Haml template with a filter, `{…}` or dynamic attributes, a Ruby block (`- if`, `each do`), lines nested under content or Ruby, a comma-continued script, `/` comments, `~`, `==`, an escaped line with interpolation, tab indentation or nested/unfinished interpolation has no Ruby mapped: the status bar reads "LSP unavailable" (not used for this template: the line and construct), Server Details says the same, and commands say so; Lenz does not check Haml syntax. ERB `%%>` in a code tag (read differently by ERB and Erubi) does the same for ERB; ERB percent-lines and stdlib trim modes are not read. The HTML server sees `_` where tags are: a tag whose output is markup (a partial, `content_tag`) is plain text to it. A type hierarchy built from syntax (for servers without one) is not offered in templates. Native macOS, Wayland and physical terminals unverified. |

## Kotlin LSP (Kotlin)

| | |
| --- | --- |
| Tested version | **Kotlin LSP 263.6379.0-EAP** (JetBrains' standalone archive `kotlin-server-263.6379.0.tar.gz`, SHA-256 checked against its published checksum; it bundles its own Java runtime; `bin/intellij-server`), Linux, 2026-10-13. Fixtures: `real_kotlin` and `real_kotlin_with_jdtls` in `crates/code-core/tests/providers.rs` (a mixed Maven project: Kotlin 2.2.21 with Kotlin and Java sources, a CRLF file with a non-BMP character, non-ASCII names), pinned in their source; `real_kotlin_through_the_window` in the GUI tests. They run only when Kotlin 2.2.21 is already in `~/.m2`. |
| Launch | `kotlin-lsp --stdio --data-sharing=none --system-path <reader cache>/kotlin-lsp/<project>-<hash>` (`kotlin-lsp` being its `bin/intellij-server`, or `executable` under `providers.kotlin-lsp`; `args` add to the fixed `--stdio --data-sharing=none`): its caches and indexes per project and launch in the reader's cache; no usage data sent. Version from `--version` (which leaves a small log in `~/.cache/JetBrains/kotlin-server2026.3`). No EULA prompt or acceptance record was involved with this build, and its license check reported the build's own EAP licence ("This build does not require a license"); the reader never accepts terms or signs in for the user. Its licence and terms beyond what it reported are unverified. |
| Languages | Kotlin (`kotlin`). Java files stay with JDT LS. |
| Workspace root | The outermost Maven/Gradle build under the browsing root (as for JDT LS), else the browsing root; overridable. A folder without a build is not imported ("no supported build system"): definitions within a file only, nothing across files (verified). |
| Readiness | Its Maven/Gradle import reports progress (shown as indexing: "Importing Maven"); ready in 1.7–1.8 s on the fixture, the index a few seconds later. Reopening files every half second during the import (an early GUI test's retry loop) made it start Maven again and again without finishing; normal browsing was not affected in the tests. |
| Operations | Definition: Kotlin to Kotlin (Unicode, CRLF), Kotlin to Java sources of the same project, also in a Kotlin-only project and after Restart Language Server. Usages of a Kotlin declaration include Java callers (its index has the project's Java sources; verified). File symbols. Workspace symbols are offered; not exercised through the reader (a direct probe listed library classes inside jars first). |
| Editing (ticket 45) | Verified on Kotlin LSP 263.6379.0-EAP (JetBrains' archive, SHA-256 checked) with `real_kotlin_editing` in `crates/code-core/tests/providers.rs`, Linux, 2026-10-25. It applies completion, quick fixes and organize imports only through its own commands (each suggestion carries an empty edit and `jetbrains.kotlin.completion.apply`; fixes `applyModCommand`; organize imports `kotlin.organize.imports`), which send the edit back with `workspace/applyEdit`. Lenz runs exactly these three commands for this server (user decision 2026-10-25) when you pick the item, and applies the edit only when it changes the open file alone at the version the command was sent for, as one Undo step; anything else is refused whole and the server is told. Other commands it offers ("Extract to local variable") stay refused. Verified: the "Import class 'LocalDate' → java.time.LocalDate" fix, completing `Random`, completing the project's Java method `twice` from Kotlin, parameter information (`make(r: Double): Círculo`), Format Document, Organize Imports removing an unused import, and a fix whose edit came back after typing refused. It answers problems through `textDocument/diagnostic` and publishes only for the first versions, so Lenz asks for each version it sends. |
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
`node --version`; saving records only `providers.pyright.node` and restarts enabled Pyright servers. Lenz Code
then prefers the matching `pyright-langserver` beside that Node installation and prepends its bin folder to
that provider's PATH. Custom executables, arguments, environment, other providers and disabled states are
preserved. **Restore automatic Node.js selection and restart** removes only this override.

A mise message such as “No version is set for shim: pyright-langserver” means the selected mise installation
cannot provide that shim; it does not prove Pyright is incompatible with the selected Node version. Choose an
installation where Pyright already exists, then check the new status. Lenz does not change mise's global
version, read shell startup files, install tools or silently retry another runtime. Java repair still changes
only JDT LS's launcher argument as described above. Other providers expose their version, failure and setup
instructions; they do not yet have runtime pickers.

## ERB editing

Ticket 58, on top of the [template routing](#templates-erb-and-haml-ruby-lsp-vs-code-html).

| | |
| --- | --- |
| Tested versions | ruby-lsp 0.26.11 with ruby-lsp-rails 0.4.8, Rails (railties, Action View) 8.1.4, sqlite3 2.10.1, RuboCop 1.92.1 and rubocop-rails 2.38.0, Ruby 3.3.6; vscode-langservers-extracted 4.10.0 on Node.js 22; **erb_lint 0.9.0** (with better_html 2.2.0 and smart_properties 1.17.0; installed for this ticket with `gem install erb_lint`); Linux, 2026-10-27. Tests: `real_rails_view_editing` (`crates/code-core/tests/providers.rs`) and `real_rails_view_assistance_matches_in_both_adapters` (`crates/code-tui/src/parity_tests.rs`), each on a Rails application the test writes (a controller, a helper, `resources :posts`, a view with CRLF and non-BMP text, a partial with strict locals, a layout); fake-server and stand-in tests `erb_templates_format_with_the_projects_erb_lint_…`, `a_rails_views_scaffolding_never_shows_…` (`crates/code-core/tests/semantics.rs`) and `template_assistance_routes_through_its_projections_alike_in_both_adapters` (both adapters). |
| Files | As for templates; Rails variants of HTML templates (`show.html+phone.erb`) serve their markup too. |
| How its Ruby runs | As Action View, Tilt and `ERB#def_method` compile a template: inside a method, so a layout's `yield` is valid (it was reported as "Invalid yield" before) and a class or constant definition is an error, as when Rails compiles it. A **Rails view** (a template under an application's `app/views`) runs in a class of its own (`LenzTemplate::View_<path>`) that is an `ActionView::Base` including `ApplicationHelper` (a mailer's views, `*_mailer/`: `ActionMailer::MailHelper`) and the helper its folder names (`posts/` → `PostsHelper`, `admin/posts/` → `Admin::PostsHelper`), in a method declared as Action View declares it (`local_assigns, output_buffer`, a block), with the view's **strict locals** (`<%# locals: (post:, note: "x") %>`, read with Action View's own pattern; `()` is `**nil`) as its parameters. Those lines are Lenz's (scaffolding): never mapped to the template, never edited, their names never shown in suggestions, file symbols or workspace symbols. Strict locals before the template's Ruby (where Rails' comment goes) are mapped, so Ruby's problems in them land on their bytes; edits inside them are refused ("inside the template's `locals:` comment"). Ruby LSP learns the view's class only by indexing the projection: Lenz asks for its document symbols right behind its opening. Server Details shows the context ("ERB template view context: a Rails view (Action View), with ApplicationHelper and PostsHelper…"). |
| Verified (real servers) | In a Rails view, through Ruby LSP's index: `link_to` (Action View) and the application's `post_link` (`PostsHelper`) completed in output tags, each one Undo step, also typed and chosen with Enter from the suggestions drawn in both adapters; parameter information `link_to(name = <default>, options = <default>, html_options = <default>, &block)` with `html_options` active (on a line holding `é𝄞`, UTF-16) and `post_link(post)`; Quick Documentation of `link_to`; Go to Definition of `post_link` (its helper) and, from the Rails add-on, of `edit_post_path` (`config/routes.rb`); a partial's strict local `note` completed; Ruby's syntax problems at the view's own `]` after CRLF and non-BMP text; a layout's `<%= yield %>` without problems; the HTML server completing `span` after `𝄞` and closing a typed `<b>` in a CRLF view. Files on disk unchanged (the booted application's SQLite `-shm`/`-wal` aside). |
| Formatting | Format Document and format-on-save run the project's **erb_lint** when an `.erb_lint.yml` (or its older `.erb-lint.yml`) at or above the template, within the open folder (else the Ruby workspace), configures it: its binstub (`bin/erb_lint`, `bin/erblint`), else `bundle exec erb_lint` when the bundle's lock lists `erb_lint`, else `erb_lint`/`erblint` on the Ruby provider's `PATH`; with `--config <file> --stdin <template> --autocorrect` from the configuration's folder, on the draft through standard input, off the UI thread, within 30 s, the result applied as one Undo step only while the draft is the version given ("the file changed while erb_lint ran" otherwise; a save waiting for it then writes nothing). erb_lint loads its configuration and the RuboCop configuration and custom linters that names, as on the command line, and writes nothing when reading standard input. Without a configuration, Format Document says "no ERB formatter is configured for this project…" and no language server is asked (Ruby LSP formats no template, the HTML server's formatting would rewrite the tags). Organize Imports is disabled: "ERB templates have no imports to organize". Server Details and Editing Preferences name the configuration and how it runs. Verified with erb_lint 0.9.0 from the application's bundle: `<%=link_to …%>` corrected to `<%= link_to … %>` in a partial with non-BMP text, and in a CRLF view (with TrailingWhitespace off), one Undo step, in both adapters (Ctrl+Alt+L). |
| erb_lint 0.9.0's own limits | Its TrailingWhitespace linter (on by default) fails on CRLF files ("begin_pos location … is out of range"): Lenz says "erb_lint could not process this template (…); nothing changed" and applies nothing (its empty output is never taken). Whenever it corrects something its parser buffer turns CRLF into LF: Lenz puts a CRLF file's line ends back, and refuses a file mixing CRLF and LF. It prints its result with a final newline (Ruby's `puts`), so a template without one gains one. It reads a template's rules by its file: an unsaved new template is not formatted ("save it first"). Its lint problems are not shown as problems (only its corrections apply). |
| Limits | **Controller instance variables** (`@posts`) are not suggested or followed: Ruby LSP's index records an instance variable on the class whose methods assign it and offers those of the receiver's class and ancestors, and a view is not its controller (Rails copies them at run time); the Rails add-on 0.4.8 provides no view assigns. **Route helpers** (`posts_path`) are not suggested (they are not in Ruby LSP's index); their Go to Definition comes from the add-on once the application has booted. **Other helpers**: Rails includes every helper in every controller's views by default (`include_all_helpers`); Lenz includes only `ApplicationHelper` and the folder's own, so other modules' helpers are not offered (nothing is offered that the view does not have). **Locals** of a partial without strict locals (passed by `render`) are not known. Ruby LSP 0.26.11 (with its Rails and RuboCop add-ons, in a `.rb` file too; reproduced by a JSON-RPC probe without Lenz) answers no definition or documentation when non-ASCII text precedes the caret on its line (`link_to("é", posts_path)`), while parameter information there works. Parameter information right after a freshly typed `(` stays empty (ticket 57). A Rails view is recognised by its `app/views` path only (a template elsewhere, a ViewComponent's sidecar template or a non-Rails `app/views` runs in a top-level method instead); a non-Rails ERB template's Ruby runs in a top-level method (Tilt's reading), so a class definition there is reported. stdlib-ERB percent lines and trim modes are not read. No syntax highlighting. Native macOS, Wayland and physical terminals unverified. |

## Haml editing

Haml (`*.haml`, including `.html.haml`) uses its own markup suggestions and indentation in both
adapters. Ctrl+Space in `%ta` suggests tag names; inside `(attr=...)`, an attribute suggestion inserts
Haml's `name=""`, preserving `%`, parentheses and surrounding text. These suggestions need no server.
Enter retains indentation, adds the configured spaces-only step after a tag or Ruby block, and dedents
an empty line. Each suggestion and Enter is one Undo step; neither writes the file.

Ruby LSP receives exact source regions from scripts, nested `if`/`else` and `do` blocks, dynamic
`{...}` attributes, variable/quoted `(…)` attributes, nested interpolation, pipe and comma continuations.
Generated block endings and hash-expression parentheses have no source coordinates. Completion, parameter
help, diagnostics and edits use the template's own URI/version and negotiated position encoding. Edits
crossing mapped regions, changing block shape or inserting template lines are refused as a whole.
For `app/views/<controller>/...`, a private per-URI class gives the projection `ActionView::Base`,
`ApplicationHelper` and the inferred controller helper as lexical context. This never boots Rails.

**Format Document** and project format-on-save run safe haml-lint autocorrections when a parent
`.haml-lint.yml` configures them. Install Ruby, `haml` and `haml_lint` yourself; Ruby is selected from
Lenz's PATH. A project locking `haml_lint` in `Gemfile.lock` uses `bundle exec`; otherwise Ruby's
installed gems are used. The draft goes through stdin, with the original filename for configuration.
The source file is never a formatter output target. Syntax/tool errors preserve the draft; Escape,
an edit, or switching away rejects the old result. Save Without Formatting remains available.
Existing CRLF/LF endings and a missing final newline survive when formatting keeps the line count;
new lines use the first existing ending. Haml-lint's explicit stdin-file behavior applies to exclusions.

Verified with Haml 7.5.1, haml_lint 0.78.0, Ruby LSP 0.26.11 and RuboCop 1.92.1 on Linux:
`real_haml_regions_are_the_ruby_haml_compiles`, `real_haml_template_editing`,
`real_haml_blocks_helpers_and_configured_formatting`, and the Haml authoring adapter replays.
The compiler/parser check verifies every mapped region and parses the generated Ruby. Fixtures cover
non-BMP characters, CRLF, nested blocks, attributes, multiline calls, helper members, parameter help,
syntax problem ranges, formatting and coherent Undo. A `File.write` sentinel remains absent during
formatting, and source files stay unchanged.

Limits: filters, object references, multiline attribute lists, tab indentation, regex/percent literals
inside mapped interpolation/attributes and unfinished interpolation remain explicitly unmapped; markup
suggestions are a finite tag/attribute list, not an HTML language server. No Haml-specific syntax
highlighting or live Haml grammar diagnostics yet; Ruby LSP reports Ruby syntax problems. Rails context
is static: controller instance-variable types, partial locals, namespaced/custom helper inference and
runtime-generated methods are unverified. Native macOS, Wayland and physical terminals remain unverified.
Performance/stress validation is deferred to the end-of-epic campaign.

## PostgreSQL offline editing

Choose **SQL Dialect for This File: PostgreSQL** or the project's PostgreSQL default. The shared
session then checks the current draft with the bundled PostgreSQL 18.4 raw parser (`pg_parse 0.15.0`).
Both adapters underline its original-source syntax range, including Unicode/CRLF and EOF. This is
syntax checking, not validation against a schema. Missing parser cursors remain an unpositioned
availability note in Problems; they are not invented locations.

Ctrl+Space completes names known from the statement's aliases, CTEs and derived tables, including
UNION outputs, recursive CTE column lists, qualified wildcards, quoted identifiers and known builtin
table-function outputs. Ambiguous unqualified columns are omitted; a non-LATERAL derived table does
not receive its enclosing FROM aliases. Other malformed statements do not hide the current statement's
scope. No external schema columns are invented. The offline catalog documents 28 common PostgreSQL
builtins; `pg_catalog` completion and Parameter Info use those documented summaries, not exhaustive
overload discovery. Commas inside arrays and parenthesized groups do not advance the enclosing argument.

**Format Document** and project format-on-save use the guarded offline whitespace formatter with
EditorConfig/project indentation. Dollar strings, significant tokens, comments and native-valid syntax
are checked before a change is accepted; rejection preserves the draft. Completion and formatting each
apply as one Undo step. Only Save writes the document.

Requests share the existing bounded analysis scheduler and one admitted document per version. Edits,
dialect or project/context changes and Escape reject obsolete results. Completion is capped at 256 KiB,
4,096 tokens and 64 expression levels. All editor SQL operations are bounded to 256 KiB. An unavailable
check leaves the text editable and says why. No language server, database connection, statement execution
or EXPLAIN is involved. A traced destructive-source fixture observed no sockets, connects or provider
child processes; source strings remained data and program/export sentinels stayed absent.

Verified through core session commands, native parser/provider fixtures, and actual GUI/TUI completion,
Parameter Info, formatting, rendering and Undo input replays. Limits: arbitrary malformed syntax recovery,
all PostgreSQL visibility edge cases, inferred types, unknown function-return columns, extension functions
and complete overload catalogs remain unsupported. Optional database metadata is a separate explicit
connection workflow; opening a file does not connect. Native macOS/Wayland and the end-of-epic performance
campaign remain unverified.

## DuckDB offline editing

Choose DuckDB in **SQL Dialect…**. Query-local completion works without a database or native tool,
including aliases, CTEs, nested/quoted names, set outputs, EXCLUDE, REPLACE and RENAME projections.
DuckDB's ASCII case-insensitive identifier lookup applies to quoted names too, while inserted names
keep their original quoting. Unknown external table/function schemas are not invented.

For native syntax checking, formatting and builtin overload help, install DuckDB yourself on PATH or
set `duckdb_executable` in Lenz's settings to an installed CLI. The SQL Dialect details show the selected
tool or why it is missing. Tested with DuckDB 2.0.0; other versions are unverified. Opening a file does
not open a metadata database: native assistance uses only an in-memory engine, one thread, a 128 MB
engine memory limit and disabled extension autoload/autoinstall/external access. Native source is capped
at 32 KiB; query-local completion retains the 256 KiB/4,096-token/64-level bounds.

The native CLI receives only fixed launch arguments. Document bytes are hex-encoded into a fixed data
expression on bounded stdin, so document newlines, quotes and line-leading `.shell`/`.read` commands
never enter the CLI command grammar. Requests have a three-second timeout, bounded output and process-group
cancellation. The entire document is parsed, including later malformed statements; the parser's exact
non-SELECT serialization limitation is classified as syntax-only success. No edited statement or EXPLAIN
runs. An observed destructive/quote/dot-command fixture retained identical database bytes and rows and
created no export/program sentinel.

Both adapters draw exact original-source diagnostic ranges, complete current-version names, show native
function parameter help and apply guarded formatting as one Undo step. Dialect, project, source, edit and
request changes reject stale responses. Native tool failures leave drafts editable; pure completion remains
available when the CLI is missing. Metadata files require a separate explicit workflow.

Limits: completion includes 21 common builtin names (native help can answer other installed builtins with
at most eight overloads); unknown table-function result schemas, arbitrary malformed lexical recovery,
complete visibility/type inference and non-ASCII case folding are not claimed. The frontend replays include
EXCLUDE/GROUP BY ALL, Unicode/CRLF, syntax errors, function help, formatting and Undo. Physical terminals,
native macOS/Wayland and end-of-epic performance remain unverified.

## PostgreSQL metadata snapshots

SQL remains offline until **Configure SQL Metadata…** and **Connect SQL Metadata** are used.
Choose PostgreSQL first, then enter a libpq service name (letters/digits, `_`, `-`, `.`),
not a URI or password. Install `psql` on PATH and keep host, database, TLS and authentication
in libpq's service/credential files. Lenz never persists the service or credentials; configuration
is scoped to this session's active project/dialect. Switching scope forgets it.

Connect and Refresh run a fixed catalog query through `psql -X -w`, in a read-only transaction,
with statement/lock deadlines, a five-second process deadline and a 1 MiB output cap. They read
user relation names, column names/types and function argument/result declarations from `pg_catalog`.
They never send the document, query rows, EXPLAIN, or call user functions. Connections close after
the snapshot is fetched. More than 512 relations or function overloads fails visibly rather than
silently truncating. Server permissions and configuration still govern which metadata is visible.

Both adapters expose the same four actions in the SQL menu. The status bar shows loading, age,
failure or off; SQL Dialect details explain the state. Qualified relation/function names are offered
from the explicit snapshot; relation aliases expose columns with type documentation. Parameter Info
uses up to eight matching function declarations. Unqualified search-path inference is not claimed.
Refresh explicitly replaces the snapshot. Disconnect cancels work and clears it. Snapshots expire
after five minutes and are rejected on scope/source changes; no automatic connection or refresh runs.
A failure clears assistance from the old snapshot and gives a credential-free error. Native connection
errors deliberately omit server output because it can contain private connection information.

## DuckDB metadata snapshots

Choose DuckDB, use **Configure SQL Metadata…** to select an existing local DuckDB or SQLite database in the GUI file chooser
(or enter its absolute path in the terminal),
then explicitly **Connect SQL Metadata**. The same SQL menu/status/details, Refresh and Disconnect
controls work in both adapters. An installed DuckDB CLI (`duckdb_executable` or PATH) is required.
Opening a document and configuring a path do not open that database. Lenz never creates a missing
metadata file. Configuration stays in this session and is forgotten when the active SQL scope changes
(project, dialect, or switching to a non-SQL document).

A separate in-memory engine disables extension autoload/installation and external access before it
attaches the chosen file **READ_ONLY**. SQLite files are recognized by their header, regardless of extension;
Lenz explicitly loads DuckDB's installed `sqlite` extension before restricting external access. If it is missing,
run `INSTALL sqlite;` in the selected DuckDB executable and reconnect. Lenz does not download extensions.
Only the selected canonical file and its WAL (plus SQLite's shared-memory sidecar) are allowed through the
engine's filesystem guard. Fixed `duckdb_columns()` / `duckdb_functions()` operations read definitions,
including macro parameters, without evaluating views/macros or executing document SQL or EXPLAIN.
The worker has one engine thread, a 128 MB engine limit, five-second deadline, 1 MiB output cap and
512-relation/function limits. Oversized catalogs fail visibly. Connection progress and errors are retained in **Logs**; DuckDB metadata failures include bounded tool diagnostics. PostgreSQL diagnostics still omit connection details. Snapshots expire after five minutes;
database or populated-WAL identity/modification changes reject them, including changes during fetch or before publication. An absent WAL and an empty WAL are equivalent: SQLite can create the empty sidecar during a read-only attachment.
Files used by another writer may fail to open; Refresh/Connect explicitly after the writer closes.

Qualified relation/function names and alias columns/types are available from the snapshot. Tables in
DuckDB/SQLite's `main` schema also complete by their unqualified names (for example `users` and `users.id`);
other schemas stay qualified. Draft `USE` or `SET search_path` statements do not change this default.
An empty completion result means no names match at that cursor position, including after a finished
statement; it does not mean the metadata connection failed. Configuration, connection results and
configuration resets are recorded in **Logs**. After selecting a database, the GUI opens the metadata
menu with **Connect SQL Metadata** selected.
Parameter Info shows declared macro parameters; DuckDB does not provide their inferred types/results without
binding, so these stay unknown/not evaluated. Builtin overloads still use the offline native helper.
The original database bytes/rows, absence of exports and read-only opens were checked with DuckDB 2.0.0,
including a filename containing an apostrophe and multiline `.shell`/`.read` drafts.
