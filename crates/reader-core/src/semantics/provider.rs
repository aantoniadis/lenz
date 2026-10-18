//! The built-in semantic providers: which languages each serves, how it is found and launched, how its
//! workspace root is resolved, and what it is known not to do. A small table, not a plugin system.

use std::path::{Path, PathBuf};

use lenz_content::highlight::Language;
use serde_json::{Value, json};

pub struct Provider {
    /// Stable identity, also the settings key.
    pub id: &'static str,
    /// The name shown to the user.
    pub name: &'static str,
    pub languages: &'static [Language],
    /// The protocol `languageId` for a file of one of `languages`.
    pub language_id: fn(Language, &Path) -> &'static str,
    /// The executable looked up on `PATH` unless the settings name another.
    pub executable: &'static str,
    pub args: &'static [&'static str],
    /// How its version is found before it starts (else from its `initialize` answer, if it gives one).
    pub version: Version,
    /// The version the reader was tested with (a prefix of what it reports); others get a notice.
    pub tested: Option<&'static str>,
    /// What to do when it is missing.
    pub setup: &'static str,
    /// The workspace root for a source file under browsing root (if any), by this provider's own discovery
    /// rules (reads manifests: call off the UI thread). `None`: it has no workspace.
    pub root: fn(&Path, Option<&Path>) -> Option<PathBuf>,
    /// Why a file has no workspace, with what to do.
    pub no_root: &'static str,
    /// Where files outside the project come from, by a path fragment: shown on them as their origin.
    pub origins: &'static [(&'static str, &'static str)],
    /// `initializationOptions` (the user's settings are merged over them).
    pub initialization: fn() -> Value,
    /// The `workspace/configuration` section its settings live under.
    pub config_section: &'static str,
    /// Client `capabilities.experimental` this provider understands.
    pub experimental: fn() -> Value,
    /// It reports readiness itself (rust-analyzer's `experimental/serverStatus`, JDT LS's `language/status`):
    /// busy until it says it is ready.
    pub reports_status: bool,
    /// The argument naming a server workspace-data folder, which the reader allocates in its cache per project
    /// and launch (JDT LS's `-data`); `None` when it keeps no such folder.
    pub workspace_data: Option<&'static str>,
    /// The documented request returning the text of a virtual (non-file) target, if the reader implements it.
    pub virtual_contents: Option<&'static str>,
    /// Known limits, as the user is told them.
    pub limits: &'static str,
    /// Variables its launch needs for workspace `root`, given its folder in the reader's cache (computed off the UI
    /// thread when it starts; the user's `env` wins).
    pub root_env: fn(&Path, &Path) -> Vec<(String, String)>,
    /// It is offered UTF-8 positions (else only UTF-16, for a server whose UTF-8 positions are wrong).
    pub utf8: bool,
    /// Arguments always passed before the launch's own (which settings may replace): what keeps it read-only and
    /// on stdio. Not part of the launch's identity (its data folder).
    pub fixed_args: &'static [&'static str],
    /// File kinds its project may hold that it does not index, and what that leaves out of its usages: said with
    /// its usages when the project has such files.
    pub blind_to: &'static [(&'static str, &'static str)],
    /// It relies on the client to report files changed on disk that it does not hold open (else it watches them
    /// itself, and is not told).
    pub watched_files: bool,
    /// Extensions of files it navigates into that no provider reads (Ruby LSP's RBS signatures): their origin is
    /// this provider's.
    pub navigates: &'static [&'static str],
    /// A server script inside VS Code's built-in extensions (relative to an extensions folder), run with `node`
    /// when the executable is not on PATH and the settings name none.
    pub bundled: Option<&'static str>,
    /// Where its executable is for workspace `root` when the settings name none (else it is looked up on PATH).
    pub locate: Option<fn(&Path, &Launch) -> Option<PathBuf>>,
}

/// JDT LS's JVM property keeping Eclipse metadata out of the project.
pub const JDT_NO_PROJECT_METADATA: &str = "--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false";

/// What a provider has unless its entry says otherwise.
const DEFAULT: Provider = Provider {
    id: "",
    name: "",
    languages: &[],
    language_id: |_, _| "plaintext",
    executable: "",
    args: &[],
    version: Version::Initialize,
    tested: None,
    setup: "",
    root: |_, _| None,
    no_root: "",
    origins: &[],
    initialization: || json!({}),
    config_section: "",
    experimental: || json!({}),
    reports_status: false,
    workspace_data: None,
    virtual_contents: None,
    limits: "",
    root_env: |_, _| Vec::new(),
    utf8: true,
    watched_files: false,
    navigates: &[],
    bundled: None,
    locate: None,
    fixed_args: &[],
    blind_to: &[],
};

pub static PROVIDERS: [Provider; 9] = [
Provider {
    id: "rust-analyzer",
    name: "rust-analyzer",
    languages: &[Language::Rust],
    language_id: |_, _| "rust",
    executable: "rust-analyzer",
    args: &[],
    version: Version::Flag(&["--version"]),
    // Its builds name a commit: `rust-analyzer 1 (f8996691e9 2026-08-30)` (Arch `rust-analyzer 20260831-1`).
    tested: Some("f8996691e9"),
    setup: "Install it with `rustup component add rust-analyzer` (or your package manager) and make sure it is on PATH; the reader never installs or upgrades it",
    root: |file, _| rust_root(file),
    no_root: "no Cargo.toml or rust-project.json encloses this file; set a workspace root",
    origins: &[("/.cargo/registry/", "a Cargo dependency"), ("/.cargo/git/", "a Cargo dependency"), ("/rustlib/src/", "the Rust standard library")],
    // The reader never builds: no cargo check on save (there are no saves), and build scripts and proc macros
    // only as rust-analyzer's defaults run them.
    initialization: || json!({ "checkOnSave": false }),
    config_section: "rust-analyzer",
    experimental: || json!({ "serverStatusNotification": true }),
    reports_status: true,
    limits: "Definitions inside macro expansions or generated code may point at sources rust-analyzer cannot show",
    // Build-script and proc-macro data are built into the reader's cache, never the project's `target/`.
    root_env: rust_env,
    ..DEFAULT
},
Provider {
    id: "pyright",
    name: "Pyright",
    languages: &[Language::Python],
    language_id: |_, _| "python",
    executable: "pyright-langserver",
    args: &["--stdio"],
    // `pyright-langserver` has no version flag and names no version when initialized: its npm package says it.
    // (Never run the `pyright` command for it: the PyPI wrapper of that name downloads Node.js and Pyright.)
    version: Version::Package("pyright"),
    tested: Some("1.1.414"),
    setup: "Install Pyright with npm (`npm install -g pyright`, which needs Node.js) so that `pyright-langserver` is on PATH, or set its `executable` and `env.PATH` in providers.json (the PyPI `pyright` package downloads Node.js and Pyright when it first runs); the reader never installs or upgrades it",
    root: python_root,
    no_root: "no Python project encloses this file; set a workspace root",
    origins: &[("/typeshed-fallback/", "Pyright's bundled type stubs"), ("/site-packages/", "an installed Python package"), ("/dist-packages/", "an installed Python package"), ("/lib/python3.", "the Python standard library")],
    // The user's settings for Pyright live under `python` (`python.analysis.*`, `python.pythonPath`).
    initialization: || json!({}),
    config_section: "python",
    limits: "Types and definitions come from Pyright's static analysis: dynamic attributes, monkey patching and unresolved imports (packages not installed in the environment it finds) are not followed; files under hidden folders are not part of its workspace by default (no workspace symbols there); usages asked at a use leave out that use",
    ..DEFAULT
},
Provider {
    id: "typescript-language-server",
    name: "typescript-language-server",
    // TSX belongs here, never to the HTML server.
    languages: &[Language::TypeScript, Language::Tsx, Language::JavaScript],
    language_id: typescript_language_id,
    executable: "typescript-language-server",
    args: &["--stdio"],
    version: Version::Flag(&["--version"]),
    tested: Some("6.0.1"),
    setup: "Install it with npm (`npm install -g typescript-language-server typescript`, which needs Node.js) so that `typescript-language-server` is on PATH, or set its `executable` and `env.PATH` in providers.json; it uses the project's own `typescript` from node_modules, else set `tsserver.path` in its `settings`; the reader never installs or upgrades it",
    root: typescript_root,
    no_root: "no TypeScript or JavaScript project encloses this file; set a workspace root",
    origins: TYPESCRIPT_ORIGINS,
    // `tsserver.path` (a TypeScript with `lib/tsserver.js`) goes in the user's settings when the project has none.
    initialization: || json!({}),
    config_section: "typescript",
    limits: "Navigation follows TypeScript's project model (tsconfig.json/jsconfig.json): files outside every project get a default one; plain JavaScript without types is resolved by inference; it needs a TypeScript with tsserver (TypeScript 6 or older: TypeScript 7 projects are served by typescript-go)",
    ..DEFAULT
},
// TypeScript 7's own language server (`tsc --lsp`), for workspaces where typescript-language-server finds no
// TypeScript with tsserver: chosen per workspace by [`serving`], never by language alone.
Provider {
    id: "typescript-go",
    name: "typescript-go",
    languages: &[Language::TypeScript, Language::Tsx, Language::JavaScript],
    language_id: typescript_language_id,
    executable: "tsc",
    args: &["--lsp", "--stdio"],
    // It names itself and its version when initialized (`typescript-go 7.0.2`).
    version: Version::Initialize,
    tested: Some("7.0.2"),
    setup: "Install TypeScript 7 in the project (`npm install --save-dev typescript@7`) or globally (`npm install -g typescript@7`, which needs Node.js) so that its `tsc` is in the project's node_modules/.bin or on PATH, or set its `executable` and `env.PATH` in providers.json; the reader never installs or upgrades it",
    root: typescript_root,
    no_root: "no TypeScript or JavaScript project encloses this file; set a workspace root",
    config_section: "typescript",
    locate: Some(go_executable),
    origins: TYPESCRIPT_ORIGINS,
    limits: "TypeScript 7's native language server, used where the project's TypeScript (else the installed one) is TypeScript 7: navigation follows TypeScript's project model (tsconfig.json/jsconfig.json); no type hierarchy from the server (the reader's own, from syntax, is used)",
    ..DEFAULT
},
Provider {
    id: "jdtls",
    name: "Eclipse JDT LS",
    languages: &[Language::Java],
    language_id: |_, _| "java",
    executable: "jdtls",
    args: &[],
    // Eclipse metadata (.project, .classpath, .settings) in its workspace-data folder, never the project: read at
    // startup, so a JVM property (as VS Code's Java extension passes it).
    fixed_args: &[JDT_NO_PROJECT_METADATA],
    blind_to: &[("kt", "Kotlin callers are not included (JDT LS does not index Kotlin)"), ("kts", "Kotlin callers are not included (JDT LS does not index Kotlin)")],
    // Its launcher has no version flag (it would start the server); it names its version when initialized.
    version: Version::Initialize,
    tested: Some("1.61.0"),
    setup: "Install Eclipse JDT LS (a `jdtls` launcher; it needs Python 3 and Java 21 or newer, from JAVA_HOME or PATH) so that `jdtls` is on PATH, or set its `executable` and `env` (PATH, JAVA_HOME) in providers.json; the reader never installs or upgrades it",
    root: java_root,
    no_root: "no Java project encloses this file; set a workspace root",
    origins: &[("/.m2/repository/", "a Maven dependency"), ("/.gradle/caches/", "a Gradle dependency")],
    // Class files without a source file open as retrieved source; the user's `settings` go under `settings`. A
    // reader never builds: no build output.
    initialization: || json!({
        "extendedClientCapabilities": { "classFileContentsSupport": true },
        "settings": { "java": { "autobuild": { "enabled": false } } },
    }),
    config_section: "java",
    reports_status: true,
    workspace_data: Some("-data"),
    virtual_contents: Some("java/classFileContents"),
    limits: "Maven and Gradle projects are imported by JDT LS itself (it may download build plugins and dependencies and write build output); folders without a build file are read as a plain source tree; Kotlin files are not part of its index",
    ..DEFAULT
},
Provider {
    id: "vscode-html",
    name: "VS Code HTML",
    languages: &[Language::Html],
    language_id: |_, _| "html",
    // vscode-langservers-extracted's command, else the server VS Code itself ships.
    executable: "vscode-html-language-server",
    args: &["--stdio"],
    version: Version::Package("vscode-langservers-extracted"),
    tested: Some("1.141.0"),
    setup: "Install VS Code (its built-in HTML server is used, with Node.js on PATH) or `npm install -g vscode-langservers-extracted` so that `vscode-html-language-server` is on PATH, or set its `executable` and `args` in providers.json; the reader never installs or upgrades it",
    root: web_root,
    no_root: "no folder holds this file; set a workspace root",
    origins: &[("/node_modules/", "an npm dependency")],
    // Its embedded <style> and <script> blocks are served too; it never formats (the reader writes nothing).
    initialization: || json!({ "provideFormatter": false, "embeddedLanguages": { "css": true, "javascript": true } }),
    config_section: "html",
    bundled: Some("html-language-features/server/dist/node/htmlServerMain.js"),
    limits: "Definitions and usages within the page's own <script> and <style> blocks only: classes, ids, links and templates are not followed to other files or frameworks, and there are no workspace symbols (find text or declarations instead); usages include the declaration",
    ..DEFAULT
},
Provider {
    id: "vscode-css",
    name: "VS Code CSS",
    languages: &[Language::Css],
    language_id: |_, _| "css",
    executable: "vscode-css-language-server",
    args: &["--stdio"],
    version: Version::Package("vscode-langservers-extracted"),
    tested: Some("1.141.0"),
    setup: "Install VS Code (its built-in CSS server is used, with Node.js on PATH) or `npm install -g vscode-langservers-extracted` so that `vscode-css-language-server` is on PATH, or set its `executable` and `args` in providers.json; the reader never installs or upgrades it",
    root: web_root,
    no_root: "no folder holds this file; set a workspace root",
    origins: &[("/node_modules/", "an npm dependency")],
    initialization: || json!({ "provideFormatter": false }),
    config_section: "css",
    bundled: Some("css-language-features/server/dist/node/cssServerMain.js"),
    limits: "Custom properties, @keyframes and other names within the same stylesheet only: selectors are not linked to HTML, templates or other stylesheets, and there are no workspace symbols (find text or declarations instead); usages include the declaration",
    ..DEFAULT
},
Provider {
    id: "ruby-lsp",
    name: "Ruby LSP",
    languages: &[Language::Ruby],
    language_id: |_, _| "ruby",
    executable: "ruby-lsp",
    args: &[],
    // `--version` prints it and exits before any bundle is composed.
    version: Version::Flag(&["--version"]),
    tested: Some("0.26.11"),
    setup: "Install it with `gem install ruby-lsp` (for your user: `--user-install`, with that gem bin folder on PATH) so that `ruby-lsp` is on PATH, or set its `executable` and `env` (PATH, GEM_HOME, a version manager's shims) in providers.json; the reader never installs or upgrades it",
    root: ruby_root,
    no_root: "no Gemfile encloses this file; set a workspace root",
    // Installed gems by their installation folders (user, system, version managers, vendored bundles), never a
    // project's own `gems/` folder; most specific first (RBS ships as a gem).
    origins: &[("/gems/rbs-", "RBS core signatures"), ("/gem/ruby/", "an installed gem"), ("/ruby/gems/", "an installed gem"), ("/bundle/ruby/", "an installed gem"), ("/lib/ruby/3.", "the Ruby standard library")],
    // No formatter or linters: the reader never formats, and RuboCop would load the project's configuration.
    initialization: || json!({ "formatter": "none", "linters": [] }),
    config_section: "rubyLsp",
    root_env: ruby_env,
    // It updates its index of closed files only from the client's reports.
    watched_files: true,
    navigates: &["rbs"],
    // Its index answers in characters even with UTF-8 negotiated (0.26.11: `Círculo` in a file not open ends a
    // byte short); characters are UTF-16 units outside the astral planes.
    utf8: false,
    limits: "Definitions and usages come from Ruby LSP's static index of the project and its installed gems: dynamically defined methods, `method_missing` and metaprogramming are not followed; method usages are matched by name (any receiver); usages include the declaration; workspace symbols match qualified names (`Shapes::Círculo`: a short name in a long namespace may not be found); gems in the Gemfile that are not installed are not indexed; it evaluates the Gemfile and loads Ruby LSP add-ons found in installed gems and in the project (`**/ruby_lsp/**/addon.rb`), as editors running it do",
    ..DEFAULT
},
Provider {
    id: "kotlin-lsp",
    name: "Kotlin LSP",
    languages: &[Language::Kotlin],
    language_id: |_, _| "kotlin",
    // JetBrains' standalone server (`bin/intellij-server` in its archive; `kotlin-lsp` is its documented command).
    executable: "kotlin-lsp",
    args: &[],
    // Its transport, and no usage data sent (its default too).
    fixed_args: &["--stdio", "--data-sharing=none"],
    version: Version::Flag(&["--version"]),
    tested: Some("263.6379.0"),
    setup: "Install JetBrains' standalone Kotlin LSP (https://github.com/Kotlin/kotlin-lsp/releases; it bundles its own Java runtime) and put its `bin/intellij-server` on PATH as `kotlin-lsp`, or set its `executable` in providers.json; the reader never installs or upgrades it",
    // Its Maven/Gradle import covers the outermost build, as JDT LS's does.
    root: java_root,
    no_root: "no Maven or Gradle project encloses this file; set a workspace root",
    origins: &[("/.m2/repository/", "a Maven dependency"), ("/.gradle/caches/", "a Gradle dependency")],
    config_section: "kotlin",
    // Its caches and indexes, per project and launch, in the reader's cache (never the user's IDE caches).
    workspace_data: Some("--system-path"),
    limits: "Kotlin files of Maven and Gradle (JVM) projects, which it imports by running the build tool (it may download plugins and dependencies); a folder without a build is not imported (definitions within a file only); Java sources of the project are part of its index (Kotlin to Java works), but Java files belong to JDT LS, which does not index Kotlin (Java to Kotlin is not followed), and the two are never combined into one answer; library sources inside jars (the Kotlin standard library) are not retrieved; an EAP build of a partly closed-source server",
    ..DEFAULT
},
];

/// Ruby LSP's bundle: the project's own Gemfile (at or above the root), else an empty one in the reader's cache. Either way
/// `BUNDLE_GEMFILE` is set, so `ruby-lsp` runs directly: it never composes a bundle in the project
/// (`.ruby-lsp/`), runs `bundle install` or updates itself.
fn ruby_env(root: &Path, cache: &Path) -> Vec<(String, String)> {
    // At or above the root: a root set inside a project keeps its bundle.
    let gemfile = root.ancestors().find_map(gemfile_in).unwrap_or_else(|| {
        let empty = cache.join("Gemfile");
        if !empty.is_file() {
            let _ = std::fs::create_dir_all(cache).and_then(|_| std::fs::write(&empty, ""));
        }
        empty
    });
    vec![("BUNDLE_GEMFILE".into(), gemfile.display().to_string())]
}

/// rust-analyzer's builds (build scripts, proc macros) go to a target folder of the root's own in the reader's
/// cache, not the project.
/// A target folder the user's own environment already names is kept.
fn rust_env(root: &Path, cache: &Path) -> Vec<(String, String)> {
    if std::env::var_os("CARGO_TARGET_DIR").is_some() {
        return Vec::new();
    }
    let name = root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    vec![("CARGO_TARGET_DIR".into(), cache.join(format!("{name}-{:016x}", fnv(&root.to_string_lossy()))).join("target").display().to_string())]
}

/// FNV-1a of `text`: stable folder names.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A bundle's Gemfile in `dir` (`Gemfile`, else `gems.rb`).
fn gemfile_in(dir: &Path) -> Option<PathBuf> {
    ["Gemfile", "gems.rb"].iter().map(|g| dir.join(g)).find(|g| g.is_file())
}

/// Ruby LSP's workspace: the nearest folder with a `Gemfile` (or `gems.rb`) under the browsing root (each bundle
/// is its own project), else that browsing root; without one, the nearest such folder, else the file's.
fn ruby_root(file: &Path, browse: Option<&Path>) -> Option<PathBuf> {
    let browse = browse.filter(|b| file.starts_with(b));
    let found = file.ancestors().skip(1).take_while(|d| browse.is_none_or(|b| d.starts_with(b))).find(|d| gemfile_in(d).is_some());
    found.or(browse).or(file.parent()).map(Path::to_owned)
}

/// Where VS Code installations keep their built-in extensions.
pub const VSCODE_EXTENSIONS: [&str; 5] = [
    "/usr/share/code/resources/app/extensions",
    "/opt/visual-studio-code/resources/app/extensions",
    "/usr/lib/code/extensions",
    "/snap/code/current/usr/share/code/resources/app/extensions",
    "/Applications/Visual Studio Code.app/Contents/Resources/app/extensions",
];

/// The first of `roots` holding the server script `relative`, with the VS Code version that ships it (from the
/// `package.json` above its extensions folder; empty when unknown). Read, never run.
pub fn bundled_server(roots: &[&str], relative: &str) -> Option<(PathBuf, String)> {
    roots.iter().map(|r| Path::new(r).join(relative)).find(|p| p.is_file()).map(|script| {
        // VS Code's own package.json is six folders up (`app/extensions/<extension>/server/dist/node/<script>`).
        let version = script.ancestors().skip(1).take(6).filter_map(|d| {
            let json: Value = serde_json::from_slice(&lenz_content::theme::read_bounded(&d.join("package.json")).ok()?).ok()?;
            (json["name"] == "Code" || json["name"] == "code-oss-dev").then(|| json["version"].as_str().map(|v| format!("VS Code {v}"))).flatten()
        }).next();
        (script, version.unwrap_or_default())
    })
}

/// HTML and CSS have no project model: the browsing root the file is in, else its folder.
fn web_root(file: &Path, browse: Option<&Path>) -> Option<PathBuf> {
    browse.filter(|b| file.starts_with(b)).map(Path::to_owned).or_else(|| file.parent().map(Path::to_owned))
}

/// How a provider's version is found before it starts.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// The executable itself, with these arguments, prints it and exits.
    Flag(&'static [&'static str]),
    /// The `version` of the npm package of this name that holds the executable (its symlinks resolved): read,
    /// never run.
    Package(&'static str),
    /// Only its `initialize` answer can say (starting it is the check).
    Initialize,
}

impl Provider {
    /// Whether `file` comes from where this provider's dependencies live (its origins: a Cargo registry or git
    /// checkout, the standard library, installed packages).
    pub fn is_dependency(&self, file: &Path) -> bool {
        let text = file.to_string_lossy();
        self.origins.iter().any(|(fragment, _)| text.contains(fragment))
    }
}

pub fn for_language(language: Language) -> Option<(usize, &'static Provider)> {
    PROVIDERS.iter().enumerate().find(|(_, p)| p.languages.contains(&language))
}

/// Pyright's project: the nearest directory with a `pyrightconfig.json`, else with a `pyproject.toml`,
/// `setup.py` or `setup.cfg`, never above the browsing root the file is in; else that browsing root; else the
/// file's directory. (A `pyrightconfig.json` higher up wins over a nearer project file, as Pyright's own
/// configuration discovery does.)
fn python_root(file: &Path, browse: Option<&Path>) -> Option<PathBuf> {
    let browse = browse.filter(|b| file.starts_with(b));
    let dirs = || file.ancestors().skip(1).take_while(move |d| browse.is_none_or(|b| d.starts_with(b)));
    dirs()
        .find(|d| d.join("pyrightconfig.json").is_file())
        .or_else(|| dirs().find(|d| ["pyproject.toml", "setup.py", "setup.cfg"].iter().any(|m| d.join(m).is_file())))
        .map(Path::to_owned)
        .or_else(|| browse.map(Path::to_owned))
        .or_else(|| file.parent().map(Path::to_owned))
}

/// Where TypeScript's dependencies come from, for both TypeScript servers (origins are looked up by language);
/// TypeScript 7 keeps its library declarations in its platform package (`@typescript/typescript-linux-x64/lib/`).
const TYPESCRIPT_ORIGINS: &[(&str, &str)] = &[("/node_modules/typescript/lib/", "TypeScript's library declarations"), ("/node_modules/@typescript/typescript-", "TypeScript's library declarations"), ("/node_modules/@types/", "a type declaration package"), ("/node_modules/", "an npm dependency")];

/// The protocol `languageId` of a TypeScript, TSX or JavaScript file.
fn typescript_language_id(language: Language, path: &Path) -> &'static str {
    match (language, path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("jsx"))) {
        (Language::Tsx, _) => "typescriptreact",
        (Language::TypeScript, _) => "typescript",
        (_, true) => "javascriptreact",
        _ => "javascript",
    }
}

/// The provider serving workspace `root` for provider `p`'s languages (`p` is [`for_language`]'s): `p` itself,
/// except that typescript-go serves TypeScript where typescript-language-server would find no TypeScript with
/// tsserver and a TypeScript 7 `tsc` can run. Reads files and looks on PATH: call off the UI thread.
pub fn serving(p: usize, root: &Path, settings: &Settings) -> usize {
    let go = || PROVIDERS.iter().position(|q| q.id == "typescript-go");
    match PROVIDERS[p].id {
        "typescript-language-server" if typescript_go_serves(&PROVIDERS[p], root, settings) => go().unwrap_or(p),
        _ => p,
    }
}

/// The provider whose settings hold `q`'s per-folder choices (disabled, workspace roots): the first for its
/// languages, so both TypeScript servers share them.
pub fn primary(q: usize) -> usize {
    PROVIDERS[q].languages.first().and_then(|l| for_language(*l)).map_or(q, |(p, _)| p)
}

/// typescript-language-server uses the tsserver its settings name, else the workspace's TypeScript, else the one
/// installed beside it; TypeScript 7 has no tsserver. typescript-go serves when none of those has one and it can run.
fn typescript_go_serves(tls: &Provider, root: &Path, settings: &Settings) -> bool {
    let Some(go) = PROVIDERS.iter().find(|q| q.id == "typescript-go") else { return false };
    if go_executable(root, &settings.launch(go)).is_none() {
        return false;
    }
    // A named tsserver it ignores when the file is not there, as it does.
    let named = settings.initialization(tls).get("tsserver").is_some_and(|t| ["path", "fallbackPath"].iter().any(|k| t.get(k).and_then(Value::as_str).is_some_and(|p| Path::new(p).is_file())));
    if named {
        return false;
    }
    if let Some(ts) = root.ancestors().map(|d| d.join("node_modules/typescript")).find(|t| t.join("package.json").is_file()) {
        return !ts.join("lib/tsserver.js").is_file();
    }
    let launch = settings.launch(tls);
    let own = find_executable(&launch.executable, launch.path()).and_then(|e| std::fs::canonicalize(e).ok());
    !own.is_some_and(|e| e.ancestors().any(|d| d.join("node_modules/typescript/lib/tsserver.js").is_file()))
}

/// typescript-go's executable for workspace `root`: the one its settings name, else a TypeScript 7 `tsc` (older
/// ones have no `--lsp`): the project's own (`node_modules/.bin`, at the root or above), else on PATH.
pub fn go_executable(root: &Path, launch: &Launch) -> Option<PathBuf> {
    if launch.executable != Path::new("tsc") {
        return find_executable(&launch.executable, launch.path());
    }
    let seven = |version: Option<String>| version.and_then(|v| v.split('.').next()?.parse::<u32>().ok()).is_some_and(|major| major >= 7);
    // A project's `.bin/tsc` may be a script (pnpm), so its version is the package's beside it.
    let local = root.ancestors().find(|d| d.join("node_modules/.bin/tsc").is_file() && seven(package_dir_version(&d.join("node_modules/typescript"))));
    local.map(|d| d.join("node_modules/.bin/tsc")).or_else(|| find_executable(Path::new("tsc"), launch.path()).filter(|e| seven(package_version(e, "typescript"))))
}

/// The `version` in the `package.json` of the npm package in `dir`.
fn package_dir_version(dir: &Path) -> Option<String> {
    let json: Value = serde_json::from_slice(&lenz_content::theme::read_bounded(&dir.join("package.json")).ok()?).ok()?;
    json["version"].as_str().map(str::to_owned)
}

/// TypeScript and JavaScript: one server for the outermost `tsconfig.json`, `jsconfig.json` or `package.json`
/// under the browsing root (tsserver finds nested project configurations itself, so nested ones share it); else
/// that browsing root; without one, the nearest such file's folder, else the file's.
fn typescript_root(file: &Path, browse: Option<&Path>) -> Option<PathBuf> {
    // A dependency (in some project's `node_modules`) belongs to that project's server, not one of its own.
    if let Some(project) = file.ancestors().find(|d| d.file_name().is_some_and(|n| n == "node_modules")).and_then(Path::parent) {
        return typescript_root(&project.join("package.json"), browse);
    }
    let browse = browse.filter(|b| file.starts_with(b));
    let marked = |d: &Path| ["tsconfig.json", "jsconfig.json", "package.json"].iter().any(|m| d.join(m).is_file());
    let mut dirs = file.ancestors().skip(1).take_while(|d| browse.is_none_or(|b| d.starts_with(b))).filter(|d| marked(d));
    match browse {
        Some(b) => Some(dirs.last().unwrap_or(b).to_owned()),
        None => dirs.next().or(file.parent()).map(Path::to_owned),
    }
}

/// JDT LS's workspace: the outermost Maven, Gradle or Eclipse project file under the browsing root (it imports
/// nested modules itself), else that browsing root; without one, the nearest, followed up through parent Maven or
/// Gradle builds; else the file's folder.
fn java_root(file: &Path, browse: Option<&Path>) -> Option<PathBuf> {
    const BUILDS: [&str; 6] = ["pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", ".project"];
    let browse = browse.filter(|b| file.starts_with(b));
    let marked = |d: &Path| BUILDS.iter().any(|m| d.join(m).is_file());
    let mut dirs = file.ancestors().skip(1).take_while(|d| browse.is_none_or(|b| d.starts_with(b))).filter(|d| marked(d));
    match browse {
        Some(b) => Some(dirs.last().unwrap_or(b).to_owned()),
        None => {
            let nearest = dirs.next();
            let parent_build = |d: &&Path| ["pom.xml", "settings.gradle", "settings.gradle.kts"].iter().any(|m| d.join(m).is_file());
            let outer = nearest.and_then(|n| n.ancestors().skip(1).take_while(parent_build).last());
            outer.or(nearest).or(file.parent()).map(Path::to_owned)
        }
    }
}

/// A stable name for a server's workspace-data folder: its root and launch (FNV-1a), so different projects or
/// launch configurations never share one.
pub fn data_folder(root: &Path, launch: &Launch) -> String {
    // An explicit serialization (not a Debug format, which may change), so the folder stays the same.
    let mut text = format!("{}\0{}", root.display(), launch.executable.display());
    for a in &launch.args {
        text += &format!("\0a{a}");
    }
    for (k, v) in &launch.env {
        text += &format!("\0e{k}={v}");
    }
    let h = fnv(&text);
    let name = root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    format!("{name}-{h:016x}")
}

/// Where a file outside the project comes from, as far as its path says, by the origins of the provider for its
/// language.
pub fn origin(path: &Path, language: Option<Language>) -> &'static str {
    let s = path.to_string_lossy();
    // A file no provider reads (RBS signatures) belongs to the provider that navigates into its kind.
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let provider = language.and_then(for_language).map(|(_, p)| p).or_else(|| PROVIDERS.iter().find(|p| p.navigates.contains(&extension)));
    provider.and_then(|p| p.origins.iter().find(|(fragment, _)| s.contains(fragment))).map_or(OUTSIDE, |(_, o)| o)
}

/// The origin of a file outside the project that no provider recognises.
pub const OUTSIDE: &str = "outside the project";

/// Whether `reported` (what a provider says its version is) is the version `tested`: a word of it (parentheses
/// aside) equal to it, or to it followed by a pre-release or build suffix.
pub fn is_tested(reported: &str, tested: &str) -> bool {
    reported.split_whitespace().any(|w| w.trim_matches(['(', ')']).trim_start_matches('v').split(['-', '+']).next() == Some(tested))
}

/// The `version` in the `package.json` of npm package `name` holding `executable` (symlinks resolved).
pub fn package_version(executable: &Path, name: &str) -> Option<String> {
    let real = std::fs::canonicalize(executable).ok()?;
    real.ancestors().skip(1).take(4).find_map(|dir| {
        let json: Value = serde_json::from_slice(&lenz_content::theme::read_bounded(&dir.join("package.json")).ok()?).ok()?;
        (json["name"] == name).then(|| json["version"].as_str().map(str::to_owned)).flatten()
    })
}

/// rust-analyzer's discovery as Cargo defines it: the nearest `rust-project.json` above any manifest; else the
/// nearest `Cargo.toml`'s package and the workspace that owns it (its `package.workspace`, else the first
/// enclosing `[workspace]` whose `members` include it and `exclude` does not); a package no workspace claims is
/// its own root.
fn rust_root(file: &Path) -> Option<PathBuf> {
    let manifest = |dir: &Path| -> Option<toml::Table> { std::fs::read_to_string(dir.join("Cargo.toml")).ok()?.parse().ok() };
    let mut dirs = file.ancestors().skip(1);
    let (package, table) = loop {
        let dir = dirs.next()?;
        if dir.join("rust-project.json").is_file() {
            return Some(dir.to_owned());
        }
        if let Some(t) = manifest(dir) {
            break (dir.to_owned(), t);
        }
    };
    if table.contains_key("workspace") {
        return Some(package);
    }
    if let Some(ws) = table.get("package").and_then(|p| p.get("workspace")).and_then(|w| w.as_str()) {
        return Some(normalize(&package.join(ws)));
    }
    for dir in package.ancestors().skip(1) {
        let Some(ws) = manifest(dir).and_then(|t| t.get("workspace").cloned()) else { continue };
        let Ok(rel) = package.strip_prefix(dir) else { break };
        let listed = |key: &str| ws.get(key).and_then(|m| m.as_array()).is_some_and(|m| m.iter().filter_map(|p| p.as_str()).any(|p| glob(p, rel)));
        // Cargo uses the first workspace above a package; one that does not list it leaves it on its own.
        return Some(if listed("members") && !listed("exclude") { dir.to_owned() } else { package });
    }
    Some(package)
}

/// Whether `path` matches a Cargo member pattern (`*` within a segment, `**` none here).
fn glob(pattern: &str, path: &Path) -> bool {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let pats: Vec<&str> = pattern.trim_end_matches('/').split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    pats.len() == parts.len() && pats.iter().zip(&parts).all(|(p, s)| segment(p, s))
}

fn segment(pattern: &str, s: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == s,
        Some((head, tail)) => s.strip_prefix(head).is_some_and(|rest| (0..=rest.len()).filter(|&i| rest.is_char_boundary(i)).any(|i| segment(tail, &rest[i..]))),
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// The executable named `name` on `path` (a `PATH` value: the launch's own, else the reader's), or `name`
/// itself when it is a path. Shell startup files are never read.
pub fn find_executable(name: &Path, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if name.components().count() > 1 {
        return name.is_file().then(|| name.to_owned());
    }
    let inherited = std::env::var_os("PATH");
    // An empty entry would mean the current directory: never searched.
    std::env::split_paths(path.or(inherited.as_deref())?).filter(|d| d.is_absolute()).map(|d| d.join(name)).find(|p| p.is_file())
}

/// How a server is launched, as settings make it: part of a server's identity, so a change starts another.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Launch {
    pub executable: PathBuf,
    pub args: Vec<String>,
    /// Variables set on top of the reader's own environment.
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// The `PATH` the server is looked up on and runs with.
    pub fn path(&self) -> Option<&std::ffi::OsStr> {
        self.env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| std::ffi::OsStr::new(v.as_str()))
    }
}

/// Explicit provider settings, kept outside any checkout: per provider, its launch (executable, arguments,
/// environment), its own settings, workspace-root overrides and the browsing roots it is disabled for.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, ProviderSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    /// Node.js used only for Pyright; leaves the original executable and environment intact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<PathBuf>,
    /// Arguments instead of the provider's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Environment variables added to the reader's (for example `PATH`, `JAVA_HOME`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,
    /// The provider's own settings: merged over its `initializationOptions` and answered to
    /// `workspace/configuration` for its section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    /// Browsing roots for which it is not used.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub disabled: std::collections::BTreeSet<PathBuf>,
    /// Browsing root → the workspace root to use for its files.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub roots: std::collections::BTreeMap<PathBuf, PathBuf>,
}

impl Settings {
    /// Settings from `file` (JSON, at most 64 KiB); a missing or malformed file is no settings.
    pub fn read(file: &Path) -> Self {
        lenz_content::theme::read_bounded(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Best effort, atomically: a failed save loses the setting, never a project file.
    pub fn save(&self, file: &Path) -> bool {
        let Some(dir) = file.parent() else { return false };
        let temp = file.with_extension(format!("tmp-{}", std::process::id()));
        let ok = std::fs::create_dir_all(dir).is_ok()
            && serde_json::to_vec_pretty(self).is_ok_and(|json| std::fs::write(&temp, json).is_ok())
            && std::fs::rename(&temp, file).is_ok();
        if !ok {
            let _ = std::fs::remove_file(&temp);
        }
        ok
    }

    /// How `provider` is launched: its defaults, overridden by these settings.
    pub fn launch(&self, provider: &Provider) -> Launch {
        let s = self.providers.get(provider.id);
        let mut launch = Launch {
            executable: s.and_then(|s| s.executable.clone()).unwrap_or_else(|| provider.executable.into()),
            args: s.and_then(|s| s.args.clone()).unwrap_or_else(|| provider.args.iter().map(|a| a.to_string()).collect()),
            env: s.map(|s| s.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default(),
        };
        if provider.id == "pyright"
            && let Some(bin) = s.and_then(|s| s.node.as_deref()).and_then(Path::parent)
        {
            if launch.executable == Path::new(provider.executable) {
                launch.executable = bin.join(if cfg!(windows) { "pyright-langserver.cmd" } else { "pyright-langserver" });
            }
            let inherited = std::env::var_os("PATH");
            let path = launch.path().or(inherited.as_deref());
            if let Ok(path) = std::env::join_paths(std::iter::once(bin.to_owned()).chain(path.into_iter().flat_map(std::env::split_paths))) {
                launch.env.retain(|(key, _)| key != "PATH");
                launch.env.push(("PATH".into(), path.to_string_lossy().into_owned()));
            }
        }
        launch
    }

    /// `provider`'s `initializationOptions`: its defaults with the user's settings merged over them, object by
    /// object (a user's `settings.java.format` keeps the default `settings.java.autobuild`).
    pub fn initialization(&self, provider: &Provider) -> serde_json::Value {
        fn merge(into: &mut serde_json::Value, from: serde_json::Value) {
            match (into, from) {
                (serde_json::Value::Object(o), serde_json::Value::Object(f)) => {
                    for (k, v) in f {
                        merge(o.entry(k).or_insert(serde_json::Value::Null), v);
                    }
                }
                (into, from) => *into = from,
            }
        }
        let mut options = (provider.initialization)();
        if let Some(user) = self.providers.get(provider.id).and_then(|s| s.settings.clone()) {
            merge(&mut options, user);
        }
        options
    }

    /// The answer to `workspace/configuration` for `section` (dotted): the provider's settings as initialized
    /// (its defaults with the user's merged over them) under it; nothing for other sections.
    pub fn configuration(&self, provider: &Provider, section: Option<&str>) -> serde_json::Value {
        let settings = self.initialization(provider);
        let rest = match section {
            None => return settings,
            Some(s) if s == provider.config_section => return settings,
            Some(s) => s.strip_prefix(provider.config_section).and_then(|r| r.strip_prefix('.')),
        };
        rest.and_then(|r| r.split('.').try_fold(&settings, |v, k| v.get(k))).cloned().unwrap_or(serde_json::Value::Null)
    }

    /// Whether `provider` is disabled for browsing root `browse`.
    pub fn disabled(&self, provider: &Provider, browse: Option<&Path>) -> bool {
        browse.is_some_and(|b| self.providers.get(provider.id).is_some_and(|s| s.disabled.contains(b)))
    }

    /// The overriding workspace root for `file` under browsing root `browse`, if one is set.
    pub fn root_override(&self, provider: &Provider, browse: Option<&Path>, file: &Path) -> Option<PathBuf> {
        let browse = browse.filter(|b| file.starts_with(b))?;
        self.providers.get(provider.id)?.roots.get(browse).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyright_node_override_selects_matching_server_and_preserves_custom_launch() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("Node with spaces/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("node"), "").unwrap();
        std::fs::write(bin.join("pyright-langserver"), "").unwrap();
        let pyright = for_language(Language::Python).unwrap().1;
        let mut s: Settings = serde_json::from_value(json!({"providers":{"pyright":{
            "node":bin.join("node"),"args":["--stdio","--custom"],
            "env":{"PATH":"/custom/bin","KEEP":"yes"}
        }}})).unwrap();
        let launch = s.launch(pyright);
        assert_eq!(launch.executable, bin.join("pyright-langserver"));
        assert_eq!(launch.args, ["--stdio", "--custom"]);
        assert!(launch.env.contains(&("KEEP".into(), "yes".into())));
        assert_eq!(std::env::split_paths(launch.path().unwrap()).collect::<Vec<_>>(), [bin.clone(), PathBuf::from("/custom/bin")]);
        s.providers.get_mut("pyright").unwrap().executable = Some("/custom/pyright".into());
        assert_eq!(s.launch(pyright).executable, PathBuf::from("/custom/pyright"));
        let ra = &PROVIDERS[0];
        s.providers.entry(ra.id.into()).or_default().env.insert("PATH".into(), "/ra/bin".into());
        s.providers.get_mut(ra.id).unwrap().node = Some(bin.join("node"));
        assert_eq!(s.launch(ra).path(), Some(std::ffi::OsStr::new("/ra/bin")));
    }

    #[test]
    fn rust_roots_follow_cargo_workspaces_and_rust_project_json() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path();
        let write = |p: &str, s: &str| {
            std::fs::create_dir_all(w.join(p).parent().unwrap()).unwrap();
            std::fs::write(w.join(p), s).unwrap();
        };
        write("Cargo.toml", "[workspace]\nmembers = [\"a\", \"crates/*\"]\nexclude = [\"crates/skip\"]\n");
        write("a/Cargo.toml", "[package]\nname = \"a\"\n");
        write("a/src/lib.rs", "");
        write("crates/b/Cargo.toml", "[package]\nname = \"b\"\n");
        write("crates/b/src/lib.rs", "");
        write("crates/skip/Cargo.toml", "[package]\nname = \"skip\"\n");
        write("crates/skip/src/lib.rs", "");
        write("solo/Cargo.toml", "[package]\nname = \"solo\"\n");
        write("solo/src/main.rs", "");
        write("far/inner/Cargo.toml", "[package]\nname = \"inner\"\nworkspace = \"../..\"\n");
        write("far/inner/src/lib.rs", "");
        write("custom/rust-project.json", "{}");
        write("custom/src/x.rs", "");
        write("loose/x.rs", "");
        assert_eq!(rust_root(&w.join("a/src/lib.rs")).as_deref(), Some(w));
        assert_eq!(rust_root(&w.join("crates/b/src/lib.rs")).as_deref(), Some(w), "a glob member");
        assert_eq!(rust_root(&w.join("crates/skip/src/lib.rs")).as_deref(), Some(&*w.join("crates/skip")), "excluded");
        assert_eq!(rust_root(&w.join("solo/src/main.rs")).as_deref(), Some(&*w.join("solo")), "not a member: its own root");
        assert_eq!(rust_root(&w.join("far/inner/src/lib.rs")).as_deref(), Some(w), "package.workspace");
        assert_eq!(rust_root(&w.join("Cargo.toml")).as_deref(), Some(w));
        assert_eq!(rust_root(&w.join("custom/src/x.rs")).as_deref(), Some(&*w.join("custom")));
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("x.rs"), "").unwrap();
        assert_eq!(rust_root(&other.path().join("x.rs")), None);
    }

    #[test]
    fn typescript_roots_share_one_server_per_browsing_root_and_name_jsx_and_tsx() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path();
        for p in ["web/package.json", "web/tsconfig.json", "web/src/a.ts", "web/packages/ui/tsconfig.json", "web/packages/ui/b.tsx", "loose/c.js"] {
            std::fs::create_dir_all(w.join(p).parent().unwrap()).unwrap();
            std::fs::write(w.join(p), "").unwrap();
        }
        assert_eq!(typescript_root(&w.join("web/packages/ui/b.tsx"), Some(w)).as_deref(), Some(&*w.join("web")), "nested configurations share the outer server");
        assert_eq!(typescript_root(&w.join("loose/c.js"), Some(w)).as_deref(), Some(w), "no project: the browsing root");
        assert_eq!(typescript_root(&w.join("web/packages/ui/b.tsx"), None).as_deref(), Some(&*w.join("web/packages/ui")), "without a browsing root: the nearest");
        let ts = &PROVIDERS[2];
        assert_eq!((ts.language_id)(Language::Tsx, Path::new("a.tsx")), "typescriptreact");
        assert_eq!((ts.language_id)(Language::JavaScript, Path::new("a.jsx")), "javascriptreact");
        assert_eq!((ts.language_id)(Language::JavaScript, Path::new("A.JSX")), "javascriptreact");
        std::fs::create_dir_all(w.join("web/node_modules/@types/x")).unwrap();
        assert_eq!(typescript_root(&w.join("web/node_modules/@types/x/index.d.ts"), None).as_deref(), Some(&*w.join("web")), "a dependency joins its project's server");
        assert_eq!(typescript_root(&w.join("web/node_modules/@types/x/index.d.ts"), Some(w)).as_deref(), Some(&*w.join("web")));
        assert_eq!((ts.language_id)(Language::JavaScript, Path::new("a.mjs")), "javascript");
        assert_eq!(for_language(Language::Tsx).map(|(_, p)| p.id), Some("typescript-language-server"));
        assert_eq!(origin(Path::new("/p/node_modules/@types/node/fs.d.ts"), Some(Language::TypeScript)), "a type declaration package");
    }

    /// Which TypeScript server serves a workspace: typescript-language-server wherever it finds a TypeScript with
    /// tsserver (the project's, else its own), typescript-go (`tsc --lsp`) where only TypeScript 7 is there.
    #[test]
    fn typescript_7_projects_and_installations_are_served_by_typescript_go() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path();
        let write = |p: &str, s: &str| {
            std::fs::create_dir_all(w.join(p).parent().unwrap()).unwrap();
            std::fs::write(w.join(p), s).unwrap();
        };
        let package = |dir: &str, version: &str| write(&format!("{dir}/package.json"), &format!("{{ \"name\": \"typescript\", \"version\": \"{version}\" }}"));
        let link = |from: &str, to: &str| {
            std::fs::create_dir_all(w.join(from).parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(w.join(to), w.join(from)).unwrap();
        };
        // An installation: typescript-language-server with TypeScript 7 beside it (no tsserver), `tsc` 7 on PATH.
        write("g/lib/node_modules/typescript-language-server/lib/cli.mjs", "");
        link("g/bin/typescript-language-server", "g/lib/node_modules/typescript-language-server/lib/cli.mjs");
        package("g/lib/node_modules/typescript", "7.0.2");
        write("g/lib/node_modules/typescript/bin/tsc", "");
        link("g/bin/tsc", "g/lib/node_modules/typescript/bin/tsc");
        // Projects: TypeScript 5 (with tsserver), TypeScript 7 (its own tsc), none.
        package("ts5/node_modules/typescript", "5.8.3");
        write("ts5/node_modules/typescript/lib/tsserver.js", "");
        package("ts7/node_modules/typescript", "7.0.2");
        write("ts7/node_modules/typescript/bin/tsc", "");
        link("ts7/node_modules/.bin/tsc", "ts7/node_modules/typescript/bin/tsc");
        std::fs::create_dir_all(w.join("plain/src")).unwrap();
        let path = w.join("g/bin").display().to_string();
        let settings = |extra: serde_json::Value| -> Settings {
            let mut all = serde_json::json!({ "typescript-language-server": { "env": { "PATH": path } }, "typescript-go": { "env": { "PATH": path } } });
            for (id, v) in extra.as_object().into_iter().flatten() {
                all[id].as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
            }
            serde_json::from_value(serde_json::json!({ "providers": all })).unwrap()
        };
        let (tls, _) = for_language(Language::TypeScript).unwrap();
        let go = PROVIDERS.iter().position(|p| p.id == "typescript-go").unwrap();
        let serving = |root: &str, s: &Settings| PROVIDERS[serving(tls, &w.join(root), s)].id;
        let s = settings(serde_json::json!({}));
        assert_eq!(serving("ts5", &s), "typescript-language-server", "the project's TypeScript has tsserver");
        assert_eq!(serving("ts7/src", &s), "typescript-go", "a TypeScript 7 project (from a folder inside it)");
        assert_eq!(serving("plain", &s), "typescript-go", "no project TypeScript, and the installed one is TypeScript 7");
        write("ts5copy/lib/tsserver.js", "");
        let configured = settings(serde_json::json!({ "typescript-language-server": { "settings": { "tsserver": { "path": w.join("ts5copy/lib/tsserver.js") } } } }));
        assert_eq!(serving("plain", &configured), "typescript-language-server", "a tsserver named in its settings");
        let gone = settings(serde_json::json!({ "typescript-language-server": { "settings": { "tsserver": { "path": "/nonexistent/tsserver.js" } } } }));
        assert_eq!(serving("plain", &gone), "typescript-go", "a named tsserver that is not there is ignored, as it does");
        assert_eq!(go_executable(&w.join("ts7"), &s.launch(&PROVIDERS[go])), Some(w.join("ts7/node_modules/.bin/tsc")), "the project's own tsc first");
        assert_eq!(go_executable(&w.join("plain"), &s.launch(&PROVIDERS[go])), Some(w.join("g/bin/tsc")));
        // typescript-language-server with a TypeScript of its own that has tsserver: it serves projects without one.
        write("g/lib/node_modules/typescript/lib/tsserver.js", "");
        assert_eq!(serving("plain", &s), "typescript-language-server");
        assert_eq!(serving("ts7", &s), "typescript-go", "the project's TypeScript decides");
        std::fs::remove_file(w.join("g/lib/node_modules/typescript/lib/tsserver.js")).unwrap();
        // No TypeScript 7 to run: typescript-language-server, which says what is missing.
        package("g/lib/node_modules/typescript", "5.8.3");
        assert_eq!(go_executable(&w.join("plain"), &s.launch(&PROVIDERS[go])), None, "`tsc` 5 has no --lsp");
        assert_eq!(serving("plain", &s), "typescript-language-server");
        assert_eq!(serving("ts7", &s), "typescript-go", "the project's own tsc 7 still runs");
        package("g/lib/node_modules/typescript", "7.0.2");
        let nowhere = settings(serde_json::json!({ "typescript-go": { "executable": "/nonexistent/tsc" } }));
        assert_eq!(serving("ts7", &nowhere), "typescript-language-server", "the named executable is not there");
        // typescript-language-server not installed: typescript-go where it can run.
        let missing = settings(serde_json::json!({ "typescript-language-server": { "executable": "/nonexistent/tls" } }));
        assert_eq!(serving("plain", &missing), "typescript-go");
        // Other languages have one provider.
        assert_eq!(super::serving(0, w, &Settings::default()), 0, "rust-analyzer");
    }

    #[test]
    fn java_roots_follow_builds_and_workspace_data_is_per_project_and_launch() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path();
        for p in ["app/pom.xml", "app/core/pom.xml", "app/core/src/main/java/A.java", "plain/src/B.java", "gr/settings.gradle", "gr/lib/build.gradle", "gr/lib/src/C.java"] {
            std::fs::create_dir_all(w.join(p).parent().unwrap()).unwrap();
            std::fs::write(w.join(p), "").unwrap();
        }
        assert_eq!(java_root(&w.join("app/core/src/main/java/A.java"), Some(w)).as_deref(), Some(&*w.join("app")), "the outermost build");
        assert_eq!(java_root(&w.join("app/core/src/main/java/A.java"), None).as_deref(), Some(&*w.join("app")), "up through parent builds");
        assert_eq!(java_root(&w.join("gr/lib/src/C.java"), None).as_deref(), Some(&*w.join("gr")), "a Gradle settings file above");
        assert_eq!(java_root(&w.join("plain/src/B.java"), Some(w)).as_deref(), Some(w), "no build: the browsing root");
        assert_eq!(java_root(&w.join("plain/src/B.java"), None).as_deref(), Some(&*w.join("plain/src")), "no build, no browsing root");
        let l = Launch { executable: "jdtls".into(), args: vec![], env: vec![] };
        let other = Launch { env: vec![("JAVA_HOME".into(), "/jdk".into())], ..l.clone() };
        assert_eq!(data_folder(Path::new("/x/app"), &l), data_folder(Path::new("/x/app"), &l), "stable");
        assert_ne!(data_folder(Path::new("/x/app"), &l), data_folder(Path::new("/y/app"), &l), "same name, another project");
        assert_ne!(data_folder(Path::new("/x/app"), &l), data_folder(Path::new("/x/app"), &other), "another launch");
        assert!(data_folder(Path::new("/x/app"), &l).starts_with("app-"));
    }

    #[test]
    fn python_roots_prefer_pyright_config_then_project_files_then_the_browsing_root() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path();
        let write = |p: &str| {
            std::fs::create_dir_all(w.join(p).parent().unwrap()).unwrap();
            std::fs::write(w.join(p), "").unwrap();
        };
        for p in ["pyproject.toml", "pkg/mod.py", "svc/pyproject.toml", "svc/app/main.py", "typed/pyrightconfig.json", "typed/sub/setup.py", "typed/sub/x.py", "loose/y.py"] {
            write(p);
        }
        assert_eq!(python_root(&w.join("pkg/mod.py"), Some(w)).as_deref(), Some(w));
        assert_eq!(python_root(&w.join("svc/app/main.py"), Some(w)).as_deref(), Some(&*w.join("svc")), "a nested project");
        assert_eq!(python_root(&w.join("typed/sub/x.py"), Some(w)).as_deref(), Some(&*w.join("typed")), "pyrightconfig.json wins");
        assert_eq!(python_root(&w.join("svc/app/main.py"), Some(&w.join("svc/app"))).as_deref(), Some(&*w.join("svc/app")), "never above the browsing root");
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("z.py"), "").unwrap();
        assert_eq!(python_root(&other.path().join("z.py"), Some(other.path())).as_deref(), Some(other.path()), "the browsing root");
        assert_eq!(python_root(&other.path().join("z.py"), None).as_deref(), Some(other.path()), "the file's folder");
        let py = Some(Language::Python);
        assert_eq!(origin(Path::new("/x/node_modules/pyright/dist/typeshed-fallback/stdlib/os/__init__.pyi"), py), "Pyright's bundled type stubs");
        assert_eq!(origin(Path::new("/usr/lib/python3.14/posixpath.py"), py), "the Python standard library");
        assert_eq!(origin(Path::new("/home/me/lib/python3x/tool.py"), py), "outside the project");
        assert_eq!(origin(Path::new("/h/.cargo/registry/src/x/lib.rs"), Some(Language::Rust)), "a Cargo dependency");
        assert_eq!(origin(Path::new("/usr/lib/python3.14/x.rs"), Some(Language::Rust)), "outside the project", "only the file's own provider's origins");
        assert!(is_tested("pyright 1.1.414", "1.1.414") && is_tested("v1.1.414-beta", "1.1.414") && !is_tested("pyright 1.1.4140", "1.1.414"));
        // An npm package's version, read through its bin symlink.
        let pkg = w.join("node_modules/pyright");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{"name":"pyright","version":"1.2.3"}"#).unwrap();
        std::fs::write(pkg.join("langserver.index.js"), "").unwrap();
        std::fs::create_dir_all(w.join("bin")).unwrap();
        std::os::unix::fs::symlink(pkg.join("langserver.index.js"), w.join("bin/pyright-langserver")).unwrap();
        assert_eq!(package_version(&w.join("bin/pyright-langserver"), "pyright").as_deref(), Some("1.2.3"));
        assert_eq!(package_version(&w.join("pkg/mod.py"), "pyright"), None);
    }

    #[test]
    fn html_and_css_have_their_own_servers_found_on_path_or_inside_vs_code() {
        assert_eq!(for_language(Language::Html).map(|(_, p)| p.id), Some("vscode-html"));
        assert_eq!(for_language(Language::Css).map(|(_, p)| p.id), Some("vscode-css"));
        assert_eq!(for_language(Language::Tsx).map(|(_, p)| p.id), Some("typescript-language-server"), "TSX stays with TypeScript");
        // No project model: the browsing root, else the file's folder.
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("site/pages/a.html");
        assert_eq!(web_root(&file, Some(d.path())), Some(d.path().to_owned()));
        assert_eq!(web_root(&file, None), Some(d.path().join("site/pages")));
        assert_eq!(web_root(&file, Some(Path::new("/elsewhere"))), Some(d.path().join("site/pages")));
        // VS Code's built-in server, with the version of the VS Code shipping it.
        let app = d.path().join("code/resources/app");
        std::fs::create_dir_all(app.join("extensions/css-language-features/server/dist/node")).unwrap();
        std::fs::write(app.join("extensions/css-language-features/server/dist/node/cssServerMain.js"), "").unwrap();
        std::fs::write(app.join("extensions/css-language-features/package.json"), r#"{"name":"css-language-features","version":"10.0.0"}"#).unwrap();
        std::fs::write(app.join("package.json"), r#"{"name":"Code","version":"1.141.0"}"#).unwrap();
        let roots = ["/nonexistent", app.join("extensions").to_str().unwrap()].map(|r| r.to_string());
        let roots: Vec<&str> = roots.iter().map(String::as_str).collect();
        let css = PROVIDERS.iter().find(|p| p.id == "vscode-css").unwrap();
        let (script, version) = bundled_server(&roots, css.bundled.unwrap()).unwrap();
        assert!(script.ends_with("cssServerMain.js") && version == "VS Code 1.141.0", "{script:?} {version}");
        assert!(is_tested(&version, css.tested.unwrap()));
        assert_eq!(bundled_server(&roots, "html-language-features/server/dist/node/htmlServerMain.js"), None);
    }

    #[test]
    fn ruby_roots_are_bundles_and_its_launch_never_composes_one_in_the_project() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("app");
        for f in ["Gemfile", "lib/a.rb", "engines/billing/Gemfile", "engines/billing/lib/b.rb", "scripts/c.rb"] {
            std::fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            std::fs::write(root.join(f), "").unwrap();
        }
        assert_eq!(ruby_root(&root.join("lib/a.rb"), Some(d.path())), Some(root.clone()));
        assert_eq!(ruby_root(&root.join("engines/billing/lib/b.rb"), Some(d.path())), Some(root.join("engines/billing")), "a nested bundle is its own project");
        std::fs::remove_file(root.join("Gemfile")).unwrap();
        assert_eq!(ruby_root(&root.join("scripts/c.rb"), Some(&root)), Some(root.clone()), "no Gemfile: the browsing root");
        assert_eq!(ruby_root(&root.join("scripts/c.rb"), None), Some(root.join("scripts")));
        // BUNDLE_GEMFILE is always set: the project's Gemfile, else an empty one in the reader's cache.
        let cache = d.path().join("cache/ruby-lsp");
        assert_eq!(ruby_env(&root.join("engines/billing"), &cache), [("BUNDLE_GEMFILE".to_string(), root.join("engines/billing/Gemfile").display().to_string())]);
        assert_eq!(ruby_env(&root, &cache), [("BUNDLE_GEMFILE".to_string(), cache.join("Gemfile").display().to_string())]);
        assert_eq!(std::fs::read(cache.join("Gemfile")).unwrap(), b"");
        assert_eq!(ruby_env(&root.join("engines/billing/lib"), &cache)[0].1, root.join("engines/billing/Gemfile").display().to_string(), "a root set inside a bundle keeps it");
        // Installed gems and RBS signatures by their install folders; a project's own `gems/` folder is not one.
        assert_eq!(origin(Path::new("/home/u/.local/share/gem/ruby/3.4.0/gems/thor-1.4.0/lib/thor.rb"), Some(Language::Ruby)), "an installed gem");
        assert_eq!(origin(Path::new("/usr/lib/ruby/gems/3.4.0/gems/rbs-4.2.0/core/file.rbs"), None), "RBS core signatures");
        assert_eq!(origin(Path::new("/usr/lib/ruby/3.4.0/set.rb"), Some(Language::Ruby)), "the Ruby standard library");
        assert_eq!(origin(Path::new("/work/gitlab/gems/gitlab-utils/lib/a.rb"), Some(Language::Ruby)), OUTSIDE);
        assert_eq!(origin(Path::new("/x/node_modules/a/b.yml"), None), OUTSIDE, "only a provider that navigates into a kind names its origin");
        assert!(!root.join(".ruby-lsp").exists() && !root.join("Gemfile").exists(), "nothing written into the project");
        assert_eq!(for_language(Language::Ruby).map(|(_, p)| p.id), Some("ruby-lsp"));
        // rust-analyzer's builds: a target folder per workspace in the cache (unless the shell names one).
        if std::env::var_os("CARGO_TARGET_DIR").is_none() {
            let env = rust_env(Path::new("/w/proj"), Path::new("/c/rust-analyzer"));
            assert_eq!(env[0].0, "CARGO_TARGET_DIR");
            assert!(env[0].1.starts_with("/c/rust-analyzer/proj-") && env[0].1.ends_with("/target"), "{env:?}");
            assert_ne!(env, rust_env(Path::new("/x/proj"), Path::new("/c/rust-analyzer")), "per workspace");
        }
        assert!(is_tested("rust-analyzer 1 (f8996691e9 2026-08-30)", "f8996691e9"), "a commit in parentheses");
        // Kotlin and Java each have their own server, never one for both.
        assert_eq!(for_language(Language::Kotlin).map(|(_, p)| p.id), Some("kotlin-lsp"));
        assert_eq!(for_language(Language::Java).map(|(_, p)| p.id), Some("jdtls"));
        let kotlin = for_language(Language::Kotlin).unwrap().1;
        assert_eq!(kotlin.workspace_data, Some("--system-path"));
        assert!(kotlin.fixed_args.contains(&"--data-sharing=none"));
        // Fixed arguments survive the user's own and are not part of the launch (its data folder stays).
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("providers.json");
        std::fs::write(&file, r#"{"providers":{"jdtls":{"args":["--jvm-arg=-Xmx2G"],"settings":{"settings":{"java":{"format":{"enabled":false}}}}}}}"#).unwrap();
        let s = Settings::read(&file);
        let jdt = for_language(Language::Java).unwrap().1;
        assert_eq!(s.launch(jdt).args, ["--jvm-arg=-Xmx2G"]);
        assert_eq!(jdt.fixed_args, [JDT_NO_PROJECT_METADATA]);
        // Settings merge object by object: the user's `settings.java.format` keeps `settings.java.autobuild`.
        let init = s.initialization(jdt);
        assert_eq!((&init["settings"]["java"]["autobuild"]["enabled"], &init["settings"]["java"]["format"]["enabled"]), (&json!(false), &json!(false)));
        assert_eq!(init["extendedClientCapabilities"]["classFileContentsSupport"], json!(true));
    }

    #[test]
    fn settings_round_trip_and_override_roots_per_browsing_root() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("cfg/providers.json");
        assert_eq!(Settings::read(&file), Settings::default());
        let mut s = Settings::default();
        s.providers.entry("rust-analyzer".into()).or_default().roots.insert("/b".into(), "/b/ws".into());
        assert!(s.save(&file));
        let s = Settings::read(&file);
        let ra = &PROVIDERS[0];
        assert_eq!(s.root_override(ra, Some(Path::new("/b")), Path::new("/b/x.rs")).as_deref(), Some(Path::new("/b/ws")));
        assert_eq!(s.root_override(ra, Some(Path::new("/b")), Path::new("/c/x.rs")), None);
        assert_eq!(s.launch(ra), Launch { executable: "rust-analyzer".into(), args: vec![], env: vec![] });
        assert!(!s.disabled(ra, Some(Path::new("/b"))));
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(Settings::read(&file), Settings::default());
    }

    #[test]
    fn launch_settings_and_configuration_come_from_the_user_settings() {
        let ra = &PROVIDERS[0];
        let s: Settings = serde_json::from_value(serde_json::json!({ "providers": { "rust-analyzer": {
            "executable": "/opt/ra", "args": ["--log"], "env": { "PATH": "/opt/bin", "RA_LOG": "info" },
            "settings": { "checkOnSave": true, "cargo": { "features": "all" } }, "disabled": ["/b"]
        }}})).unwrap();
        let l = s.launch(ra);
        assert_eq!((l.executable, l.args), (PathBuf::from("/opt/ra"), vec!["--log".to_string()]));
        assert_eq!(s.launch(ra).path(), Some(std::ffi::OsStr::new("/opt/bin")));
        assert_eq!(s.initialization(ra), serde_json::json!({ "checkOnSave": true, "cargo": { "features": "all" } }), "the user's settings win");
        assert_eq!(s.configuration(ra, Some("rust-analyzer")), serde_json::json!({ "checkOnSave": true, "cargo": { "features": "all" } }));
        assert_eq!(s.configuration(ra, Some("rust-analyzer.cargo")), serde_json::json!({ "features": "all" }));
        assert_eq!(s.configuration(ra, Some("rust-analyzer.missing")), serde_json::Value::Null);
        assert_eq!(s.configuration(ra, Some("editor")), serde_json::Value::Null);
        assert_eq!(s.configuration(ra, Some("rust-analyzercargo")), serde_json::Value::Null, "a section boundary, not a prefix");
        // Defaults are answered too once the user sets something else.
        let only: Settings = serde_json::from_value(serde_json::json!({ "providers": { "rust-analyzer": { "settings": { "cargo": {} } } } })).unwrap();
        assert_eq!(only.configuration(ra, Some("rust-analyzer"))["checkOnSave"], false);
        assert_eq!(find_executable(Path::new("tool"), Some(std::ffi::OsStr::new(""))), None, "an empty PATH entry is not the current directory");
        assert!(s.disabled(ra, Some(Path::new("/b"))) && !s.disabled(ra, Some(Path::new("/c"))));
        // A PATH of the launch's own, without reading shell startup files.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("tool"), "").unwrap();
        assert_eq!(find_executable(Path::new("tool"), Some(d.path().as_os_str())), Some(d.path().join("tool")));
        assert_eq!(find_executable(Path::new("tool"), Some(std::ffi::OsStr::new("/nonexistent"))), None);
    }
}
