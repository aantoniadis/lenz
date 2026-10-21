# Large-workspace LSP practices applicable to Lenz

Researched 2026-10-21. This is source research, not an editor-to-editor benchmark. Ruby LSP server and its bundled VS Code extension source are pinned to the `v0.26.11` commit `6745c162fce98b1738f8f602fbca38806c6863aa`; contemporary Zed Ruby extension, Helix and Neovim sources are pinned separately below. Their newer defaults are not assumed to exist in Ruby LSP 0.26.11. Existing Lenz measurements are in [performance](../code/performance.md#large-rails-indexing-experiment-2026-10-21).

## What transfers

1. **Configure the server's index explicitly.** The Ruby LSP VS Code extension exposes `excludedPatterns` for project files and `excludedGems` for dependencies, warning that these affect navigation and completion. Its README gives RuboCop gems as an exclusion example; these are examples, not automatic exclusions. The client forwards indexing options to the server ([pinned client](https://github.com/Shopify/ruby-lsp/blob/6745c162fce98b1738f8f602fbca38806c6863aa/vscode/src/client.ts#L232), [official extension example](https://github.com/Shopify/ruby-lsp/blob/abc74fa15864fd5dd65731a02d1f9a3e4214f36d/vscode/README.md#indexing-configuration)). A gems-only experiment retains project migrations, scripts and test support, unlike the previous combined exclusions.

2. **Use one primary navigation server and reuse it by project.** Zed's Ruby documentation selects Ruby LSP or Solargraph, with explicit examples disabling the other server and optionally RuboCop ([Zed Ruby](https://zed.dev/docs/languages/ruby#language-servers)). Neovim documents on-demand startup for matching filetypes and reuse by server name and root/workspace folders ([pinned Neovim documentation](https://github.com/neovim/neovim/blob/1944cb3201a5e729dbc4b0d614dfd355fd01620f/runtime/doc/lsp.txt#L1360)). Helix offers root markers and explicit relative `workspace-lsp-roots` for workspace boundaries ([Helix language configuration](https://docs.helix-editor.com/languages.html)). This does not justify pointing a Rails server at `app/`: it needs the complete bundle/application root for correct project answers.

3. **Keep project tuning scoped to the project.** VS Code supports workspace settings; Zed accepts server initialization options in `.zed/settings.json`; Helix merges `.helix/languages.toml` with user and built-in settings ([VS Code settings](https://code.visualstudio.com/docs/configure/settings#_workspace-settings), [Zed Ruby configuration](https://zed.dev/docs/languages/ruby#setting-up-ruby-lsp), [Helix configuration](https://docs.helix-editor.com/languages.html#languagestoml-files)). At the research baseline, Lenz exposed provider-wide initialization settings plus per-browsing-root root overrides and disabling, but no per-project initialization settings ([Lenz settings](../../crates/reader-core/src/semantics/provider.rs)). Copying Discourse's exclusions into global settings would also affect unrelated Ruby projects. The implemented [project override](../code/providers.md#project-language-server-settings) lives in Lenz's existing user-owned configuration without automatically executing checkout configuration.

## Ruby LSP 0.26.11 details that change the recommendation

The [pinned index configuration](https://github.com/Shopify/ruby-lsp/blob/6745c162fce98b1738f8f602fbca38806c6863aa/lib/ruby_indexer/lib/ruby_indexer/configuration.rb) establishes these facts:

- Dependencies whose groups are exactly `[:development]`, and eligible transitive dependencies, are excluded by default. A gem in both development and test is not excluded by that condition. `includedGems` can restore excluded dependencies.
- Common test/spec filenames and fixtures already have exclusion patterns. Top-level `tmp`, `node_modules` and `sorbet` are skipped during initial directory selection. Bundler's configured installation path has a project-file exclusion.
- `includedPatterns` appends to defaults; it is not an allowlist. Workspace patterns are expanded before excluded files are filtered, so exclusions save parsing without necessarily saving traversal.
- Gem paths are added after project exclusion filtering; use `excludedGems` for them. `.gitignore`, editor search exclusions and explorer visibility are not inputs to this method.

The deprecated `.index.yml`, if present, takes precedence over initialization indexing settings: the server returns after loading it ([pinned server](https://github.com/Shopify/ruby-lsp/blob/6745c162fce98b1738f8f602fbca38806c6863aa/lib/ruby_lsp/server.rb#L1340)). Check for this before concluding a settings experiment failed.

Add-ons are separately discovered from gems and project paths and then activated. Index exclusions are not an add-on disable mechanism; no universal `addonSettings.enable = false` switch is established by this source ([pinned add-on loader](https://github.com/Shopify/ruby-lsp/blob/6745c162fce98b1738f8f602fbca38806c6863aa/lib/ruby_lsp/addon.rb#L52)).

## What does not transfer automatically

- **Watcher exclusions are not index exclusions.** VS Code documents `files.watcherExclude` as reducing recursive file-event overhead ([VS Code API](https://code.visualstudio.com/api/references/vscode-api#workspace.createFileSystemWatcher)). Excluding an indexed source from change notifications can leave answers stale. Ruby LSP's initial file discovery is a separate operation. Hidden/search-excluded files must not be assumed absent from its index.
- **Caches belong to the particular server.** clangd checks and writes disk index shards to avoid reindexing unchanged inputs ([clangd index design](https://clangd.llvm.org/design/indexing#backgroundindex)). That is clangd behavior, not a portable LSP cache setting. Ruby LSP 0.26.11's inspected `index_all` builds an in-memory index and skips repeat indexing within that instance; the documented indexing settings expose no equivalent persistent-index switch ([pinned index implementation](https://github.com/Shopify/ruby-lsp/blob/6745c162fce98b1738f8f602fbca38806c6863aa/lib/ruby_indexer/lib/ruby_indexer/index.rb#L357)).
- **Correct Ruby/Bundler context is primarily a correctness requirement.** Official Ruby LSP setup requires the project's Ruby and Bundler environment. Zed prefers `bundle exec` when the server is in the Gemfile, otherwise PATH ([Ruby LSP editor setup](https://shopify.github.io/ruby-lsp/editors.html), [Zed activation](https://zed.dev/docs/languages/ruby#language-server-activation)). VS Code supports a custom bundle or composed bundle ([pinned launch logic](https://github.com/Shopify/ruby-lsp/blob/6745c162fce98b1738f8f602fbca38806c6863aa/vscode/src/client.ts#L80)). Lenz intentionally sets `BUNDLE_GEMFILE` and suppresses composition/install/update; changing that is not an established speedup ([Lenz Ruby environment](../../crates/reader-core/src/semantics/provider.rs)).

## Small experiments justified by these sources

1. Repeat **baseline versus tooling-gems-only exclusions** without Ruby profiling hooks, with the same fixture, bundle and warm OS cache. Preserve default project-file coverage. Record ready time, peak process-tree RSS and application/dependency navigation checks. Explicitly report lost tooling-source coverage.
2. Exercise **repeated file opening and switching under one bundle root**, asserting one initial indexing cycle and a stable server process. Lenz already uses a provider/root/launch key, starts on document demand, and retains idle servers for 600 seconds ([shared semantics](../../crates/reader-core/src/semantics/mod.rs)). A regression here would be a bug to fix, not a reason to add another reuse abstraction. Separate windows and fresh application launches are different lifetimes.
3. Only if profiling identifies unused sibling projects, compare **the correct application root versus an unnecessarily broad parent root** using existing root overrides. Confirm cross-file and gem navigation. This is conditional; narrowing the root below the application/bundle boundary can omit required source.

No editor binaries, native macOS behavior, Rails add-on boot, or newer Ruby LSP indexing engines were benchmarked for this note. Source-backed practices establish candidates and semantic limits, not an expected percentage improvement.

## Local follow-up

The [shared-core/provider trial](../code/performance.md#editor-lsp-practices-trial-2026-10-21) subsequently
measured baseline versus tooling-only exclusions on Discourse and exercised a stable server over repeated
file switches. Readiness improved 13.33→11.94 seconds with default project-file coverage preserved; both application and
Rails-gem navigation checks passed. One Ruby LSP process served 1,062 switches and file-symbol requests in
30.082 seconds. These are Linux Lenz measurements, not performance measurements of the other editors.


The subsequent [cache prototype trial](../code/performance.md#persistent-index-prototypes-2026-10-21)
selected persistent local declarations with full content validation for further development. Real Session
local coverage improved 7.89→0.49 seconds on reopening; Ruby LSP Ready improved 14.45→13.55 seconds.
A faster gem-record replay prototype failed a Ruby class-reopening correctness check and was rejected.
These are isolated prototypes, not current product behavior; memory and production integration remain open.
