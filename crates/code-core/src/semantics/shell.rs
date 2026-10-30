//! Shell scripts: which shell a script is written for, and the static tools Lenz runs on it itself. bash-language-server
//! (names, completion, documentation) is told never to run ShellCheck or shfmt: 5.8.1 checks a script without a shebang
//! as Bash whatever its project's `.shellcheckrc` says, and places ShellCheck's columns (characters) as UTF-16 units, a
//! column short after each character outside the BMP. Lenz identifies the shell itself, in the order [`ORDER`] gives,
//! runs ShellCheck as that shell on the text shown (on standard input, off the UI thread, after edits pause) and shfmt
//! as that shell for Format Document. Neither tool runs or sources the script; ShellCheck reads the files it sources
//! as text, as it does on the command line.
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    ops::Range,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{SourceVersion, diagnostics::{Diagnostic, FileDiagnostics, Severity}};
use crate::{document::Document, session::Completion, text::TextEdit};

/// The shells Lenz tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell { Sh, Bash, Dash, Ksh, Zsh }

impl Shell {
    pub fn label(self) -> &'static str {
        match self { Shell::Sh => "sh", Shell::Bash => "Bash", Shell::Dash => "dash", Shell::Ksh => "ksh", Shell::Zsh => "zsh" }
    }
    /// ShellCheck's `--shell` for it (ShellCheck does not check zsh).
    pub fn shellcheck(self) -> Option<&'static str> {
        match self { Shell::Sh => Some("sh"), Shell::Bash => Some("bash"), Shell::Dash => Some("dash"), Shell::Ksh => Some("ksh"), Shell::Zsh => None }
    }
    /// shfmt's `--language-dialect` for it (shfmt 3.8 does not parse zsh).
    pub fn shfmt(self) -> Option<&'static str> {
        match self { Shell::Sh | Shell::Dash => Some("posix"), Shell::Bash => Some("bash"), Shell::Ksh => Some("mksh"), Shell::Zsh => None }
    }
    /// A shell by an interpreter's or a setting's name, versions aside (`ksh93`, `bash5`): `posix` is sh, `mksh` and
    /// the other Korn shells ksh, `ash` and BusyBox's sh dash (ShellCheck 0.9 has no BusyBox dialect; they share
    /// dash's ancestry), Bats files Bash.
    pub fn named(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        Some(match name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.') {
            "sh" | "posix" => Shell::Sh,
            "bash" | "bats" => Shell::Bash,
            "dash" | "ash" | "busybox" => Shell::Dash,
            "ksh" | "mksh" | "pdksh" | "oksh" | "lksh" => Shell::Ksh,
            "zsh" => Shell::Zsh,
            _ => return None,
        })
    }
}

/// What named the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Basis {
    /// A `# shellcheck shell=…` directive before the script's first command.
    Directive,
    /// The first line, as written.
    Shebang(String),
    /// A file name or extension that names one shell (`.bash`, `.profile`).
    Name(String),
    /// The `shell` key of bash-language-server's provider settings (per project, else global).
    Setting,
    /// `shell=` in the nearest `.shellcheckrc` above the file.
    ShellcheckRc(PathBuf),
    /// EditorConfig's `shell_variant` (shfmt's key).
    EditorConfig(PathBuf),
    /// Nothing did: Bash.
    Default,
}

/// The shell a script is identified as, and why; `Err` holds a name that is no shell Lenz checks (`python3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identified {
    pub shell: Result<Shell, String>,
    pub basis: Basis,
}

/// Where the shell comes from, first match winning, as the user is told it.
pub const ORDER: &str = "the file's `# shellcheck shell=` directive, its shebang, a file name or extension that names one shell (.bash, .ksh, .zsh, .bashrc, .profile…; .sh names none), the `shell` setting of bash-language-server in providers.json (per project, else global), `shell=` in the nearest .shellcheckrc, EditorConfig's `shell_variant`, else Bash";

impl Identified {
    pub fn describe(&self) -> String {
        let basis = match &self.basis {
            Basis::Directive => "the file's `# shellcheck shell=` directive".to_owned(),
            Basis::Shebang(line) => format!("the shebang `{line}`"),
            Basis::Name(name) => format!("its name ({name})"),
            Basis::Setting => "the `shell` setting of bash-language-server in providers.json".to_owned(),
            Basis::ShellcheckRc(file) => format!("`shell=` in {}", file.display()),
            Basis::EditorConfig(file) => format!("`shell_variant` in {}", file.display()),
            Basis::Default => return "Bash: nothing names the shell (no directive, shebang, shell file name, setting, .shellcheckrc or EditorConfig `shell_variant`), and Bash is what ShellCheck and bash-language-server assume".to_owned(),
        };
        match &self.shell {
            Ok(shell) => format!("{}, from {basis}", shell.label()),
            Err(name) => format!("{name}, from {basis}: not a shell Lenz checks or formats (sh, Bash, dash, ksh, zsh)"),
        }
    }
}

/// How much of a script's start is read for its shebang and directives.
const HEAD: usize = 4096;

/// The start of `text` that identification reads.
pub fn head(text: &str) -> &str {
    let mut end = text.len().min(HEAD);
    while !text.is_char_boundary(end) { end -= 1; }
    text.get(..end).unwrap_or_default()
}

/// The shell of `file`, whose text starts with `head`, by [`ORDER`]; `setting` is the provider setting. Reads the
/// nearest `.shellcheckrc` and the `.editorconfig` files above it, only when the script itself names no shell: call
/// off the UI thread.
pub fn identify(file: &Path, head: &str, setting: Option<&str>) -> Identified {
    let quick = identify_in(head, file, setting, None, None);
    if quick.basis != Basis::Default { return quick; }
    let rc = shellcheckrc(file);
    let variant = crate::preferences::editorconfig_properties(file).remove("shell_variant");
    identify_in(head, file, setting, rc.as_ref().map(|(v, p)| (v.as_str(), p.as_path())), variant.as_ref().map(|(v, p)| (v.as_str(), p.as_path())))
}

/// [`identify`] with the project files' values given: `rc` is `.shellcheckrc`'s `shell=` and `variant` EditorConfig's
/// `shell_variant`, each with its file.
pub(crate) fn identify_in(head: &str, file: &Path, setting: Option<&str>, rc: Option<(&str, &Path)>, variant: Option<(&str, &Path)>) -> Identified {
    let named = |name: &str| Shell::named(name).ok_or_else(|| name.to_owned());
    if let Some(name) = directive(head) { return Identified { shell: named(&name), basis: Basis::Directive }; }
    if let Some((line, program)) = shebang(head.lines().next().unwrap_or_default()) { return Identified { shell: named(&program), basis: Basis::Shebang(line) }; }
    if let Some((name, shell)) = by_name(file) { return Identified { shell: Ok(shell), basis: Basis::Name(name) }; }
    if let Some(name) = setting.map(str::trim).filter(|s| !s.is_empty()) { return Identified { shell: named(name), basis: Basis::Setting }; }
    if let Some((value, from)) = rc { return Identified { shell: named(value), basis: Basis::ShellcheckRc(from.to_owned()) }; }
    if let Some((value, from)) = variant.filter(|(v, _)| *v != "auto") { return Identified { shell: named(value), basis: Basis::EditorConfig(from.to_owned()) }; }
    Identified { shell: Ok(Shell::Bash), basis: Basis::Default }
}

/// `shell=` of a `# shellcheck …` directive among the comments and blank lines before the first command (after the
/// shebang), where ShellCheck applies it to the whole file.
fn directive(head: &str) -> Option<String> {
    for (i, line) in head.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || (i == 0 && line.starts_with("#!")) { continue; }
        let mut words = line.strip_prefix('#')?.split_whitespace();
        if words.next() != Some("shellcheck") { continue; }
        if let Some(value) = words.find_map(|w| w.strip_prefix("shell=")) { return Some(value.to_owned()); }
    }
    None
}

/// The first line as written and the program it runs: `#!/bin/sh -e` is `sh`, `#!/usr/bin/env -S bash -x` `bash`.
fn shebang(first: &str) -> Option<(String, String)> {
    let mut words = first.strip_prefix("#!")?.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    if program == "env" {
        program = words.find(|w| !w.starts_with('-') && !w.contains('='))?.rsplit('/').next()?;
    }
    Some((first.trim_end().to_owned(), program.to_owned()))
}

/// A file name or extension that names one shell; `.sh` names none (Bash and sh scripts both use it).
fn by_name(file: &Path) -> Option<(String, Shell)> {
    let name = file.file_name()?.to_str()?.to_ascii_lowercase();
    let shell = match name.as_str() {
        ".bashrc" | ".bash_profile" | ".bash_aliases" | ".bash_logout" | "pkgbuild" | "apkbuild" => Shell::Bash,
        ".profile" => Shell::Sh,
        ".zshrc" | ".zshenv" | ".zprofile" => Shell::Zsh,
        _ => {
            let (_, extension) = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty())?;
            let shell = match extension { "bash" | "bats" => Shell::Bash, "ksh" | "mksh" => Shell::Ksh, "dash" => Shell::Dash, "zsh" => Shell::Zsh, _ => return None };
            return Some((format!(".{extension}"), shell));
        }
    };
    Some((name, shell))
}

/// `shell=` in the nearest `.shellcheckrc` (or `shellcheckrc`) above `file`, as ShellCheck finds it from the script's
/// folder; the nearest file is the one ShellCheck reads, whether it names a shell or not.
fn shellcheckrc(file: &Path) -> Option<(String, PathBuf)> {
    for dir in file.ancestors().skip(1) {
        for name in [".shellcheckrc", "shellcheckrc"] {
            let path = dir.join(name);
            let Ok(bytes) = lenz_content::theme::read_bounded(&path) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let shell = text.lines().map(|l| l.split('#').next().unwrap_or_default()).flat_map(str::split_whitespace).filter_map(|w| w.strip_prefix("shell=")).next_back()?;
            return Some((shell.to_owned(), path));
        }
    }
    None
}

/// A tool Lenz runs, as found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tool {
    Found { path: PathBuf, version: Option<String> },
    /// Not found by this name on `PATH` (or at this path).
    Missing(String),
    /// Turned off: its setting is empty.
    Off,
}

impl Tool {
    /// `name 0.9.0 (/usr/bin/name)`, or why it is not used.
    pub fn identity(&self, name: &str) -> String {
        match self {
            Tool::Found { path, version } => format!("{name} {} ({})", version.as_deref().unwrap_or("of unknown version"), path.display()),
            Tool::Missing(named) if named.contains('/') => format!("{name} is missing ({named} does not exist)"),
            Tool::Missing(named) => format!("{name} is missing (`{named}` is not on PATH)"),
            Tool::Off => format!("{name} is turned off (its path is empty in the provider settings)"),
        }
    }
}

const SHELLCHECK_SETUP: &str = "Install it with your package manager (`apt install shellcheck`, `brew install shellcheck`) or from https://github.com/koalaman/shellcheck/releases, or set `shellcheckPath` in bash-language-server's provider settings; Lenz never installs it";
const SHFMT_SETUP: &str = "Install it with your package manager (`apt install shfmt`, `brew install shfmt`) or `go install mvdan.cc/sh/v3/cmd/shfmt@latest`, or set `shfmt.path` in bash-language-server's provider settings; Lenz never installs it";

/// ShellCheck as bash-language-server's settings name it (`shellcheckPath`, else `shellcheck` on `path`).
pub fn shellcheck(settings: &Value, path: Option<&OsStr>) -> Tool {
    tool(settings.get("shellcheckPath"), "shellcheck", path, |out| out.lines().find_map(|l| l.strip_prefix("version:")).map(|v| v.trim().to_owned()))
}

/// shfmt as bash-language-server's settings name it (`shfmt.path`, else `shfmt` on `path`).
pub fn shfmt(settings: &Value, path: Option<&OsStr>) -> Tool {
    tool(settings.get("shfmt").and_then(|s| s.get("path")), "shfmt", path, |out| out.lines().next().map(|l| l.trim().trim_start_matches('v').to_owned()).filter(|v| !v.is_empty()))
}

fn tool(configured: Option<&Value>, default: &str, path: Option<&OsStr>, parse: fn(&str) -> Option<String>) -> Tool {
    let name = match configured.and_then(Value::as_str).map(str::trim) {
        Some("") => return Tool::Off,
        Some(name) => name,
        None => default,
    };
    match super::provider::find_executable(Path::new(name), path) {
        Some(found) => { let version = version(&found, parse); Tool::Found { path: found, version } }
        None => Tool::Missing(name.to_owned()),
    }
}

/// A tool's `--version`, once per executable and modification time.
fn version(exe: &Path, parse: fn(&str) -> Option<String>) -> Option<String> {
    type Seen = Mutex<HashMap<PathBuf, (Option<std::time::SystemTime>, Option<String>)>>;
    static SEEN: OnceLock<Seen> = OnceLock::new();
    let stamp = std::fs::metadata(exe).and_then(|m| m.modified()).ok();
    let seen = SEEN.get_or_init(Default::default);
    if let Some((s, v)) = seen.lock().ok()?.get(exe) && *s == stamp { return v.clone(); }
    let dir = exe.parent().unwrap_or(Path::new("/"));
    let found = super::runtime::run_with_input(exe, &["--version".to_owned()], "", dir, Duration::from_secs(5)).ok().as_deref().and_then(parse);
    seen.lock().ok()?.insert(exe.to_owned(), (stamp, found.clone()));
    found
}

/// One of ShellCheck's fixes for a problem, as edits of the version it checked (or why it cannot apply).
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    /// The problem's range.
    pub range: Range<usize>,
    pub title: String,
    pub edits: Result<Vec<TextEdit>, &'static str>,
}

/// One check of one version of a script: its shell, the tools found, and ShellCheck's problems and fixes.
#[derive(Debug)]
pub struct Lint {
    pub shell: Identified,
    pub shellcheck: Tool,
    pub shfmt: Tool,
    /// ShellCheck's problems for exactly the version checked; `None` when it did not check it.
    pub diagnostics: Option<Arc<FileDiagnostics>>,
    pub fixes: Vec<Fix>,
    /// Why ShellCheck did not check this version, as the user is told it.
    pub unchecked: Option<String>,
}

impl Lint {
    /// What the problems list says when ShellCheck did not check the text.
    pub fn note(&self) -> Option<String> {
        if self.diagnostics.is_some() { return None; }
        Some(match &self.shellcheck {
            Tool::Found { .. } => format!("ShellCheck did not check this text: {}.", self.unchecked.as_deref().unwrap_or("unknown reason")),
            Tool::Missing(_) => format!("{}, so no ShellCheck diagnostics are available. {SHELLCHECK_SETUP}.", self.shellcheck.identity("ShellCheck")),
            Tool::Off => format!("{}, so no ShellCheck diagnostics are available.", self.shellcheck.identity("ShellCheck")),
        })
    }

    /// Server Details' rows for the script.
    pub fn details(&self) -> Vec<String> {
        let shell = self.shell.shell.as_ref().ok().copied();
        let problems = match (&self.shellcheck, &self.unchecked) {
            (Tool::Found { .. }, None) => format!("Problems: {}, checking as {}; it reads the text shown on standard input, never runs or sources the script, and runs again when edits pause", self.shellcheck.identity("ShellCheck"), shell.and_then(Shell::shellcheck).unwrap_or("?")),
            (Tool::Found { .. }, Some(why)) => format!("Problems: {} did not check this text: {why}", self.shellcheck.identity("ShellCheck")),
            (Tool::Missing(_), _) => format!("Problems: {}, so no ShellCheck diagnostics are available. {SHELLCHECK_SETUP}", self.shellcheck.identity("ShellCheck")),
            (Tool::Off, _) => format!("Problems: {}, so no ShellCheck diagnostics are available", self.shellcheck.identity("ShellCheck")),
        };
        let formatting = match (&self.shfmt, shell.and_then(Shell::shfmt)) {
            (Tool::Found { .. }, Some(dialect)) => format!("Format Document: {}, as `-ln={dialect}` with this file's indentation and its EditorConfig shfmt options (else the provider's `shfmt` settings)", self.shfmt.identity("shfmt")),
            (Tool::Found { .. }, None) => format!("Format Document: {} does not format {}", self.shfmt.identity("shfmt"), shell.map_or("this script", Shell::label)),
            (Tool::Missing(_), _) => format!("Format Document: {}. {SHFMT_SETUP}", self.shfmt.identity("shfmt")),
            (Tool::Off, _) => format!("Format Document: {}", self.shfmt.identity("shfmt")),
        };
        vec![
            format!("Shell: {}. Lenz identifies it from {ORDER}.", self.shell.describe()),
            problems,
            if self.shell.shell == Ok(Shell::Zsh) { "Built-in syntax: zsh is not checked by the Bash grammar".into() }
                else { "Built-in syntax: structural problems are checked separately by the Bash-compatible grammar, including when ShellCheck is missing or disabled".into() },
            formatting,
            "Problems, quick fixes and Format Document for shell scripts come from ShellCheck and shfmt, which Lenz runs itself; bash-language-server is told not to run them. Typing, Undo and Save never wait for them.".to_owned(),
        ]
    }
}

/// How long one ShellCheck run may take.
const LINT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long edits must pause before the text is checked.
const PAUSE: Duration = Duration::from_millis(300);

/// Check `doc` (its file `doc.source()`) with bash-language-server's merged `settings` and the `PATH` tools are looked
/// up on. Reads project files and runs ShellCheck: off the UI thread.
pub fn lint(doc: &Document, settings: &Value, path: Option<&OsStr>) -> Lint {
    let shell = identify(doc.source(), head(doc.text()), settings.get("shell").and_then(Value::as_str));
    let (shellcheck, shfmt) = (shellcheck(settings, path), shfmt(settings, path));
    let mut lint = Lint { shell, shellcheck, shfmt, diagnostics: None, fixes: Vec::new(), unchecked: None };
    let Tool::Found { path: exe, .. } = lint.shellcheck.clone() else { return lint };
    let dialect = match &lint.shell.shell {
        Ok(shell) => match shell.shellcheck() { Some(d) => d, None => { lint.unchecked = Some(format!("ShellCheck does not check {} scripts", shell.label())); return lint; } },
        Err(name) => { lint.unchecked = Some(format!("{name} is not a shell ShellCheck checks")); return lint; }
    };
    match check(&exe, doc, dialect, settings) {
        Ok((items, fixes)) => { lint.diagnostics = Some(Arc::new(FileDiagnostics::new(doc, items))); lint.fixes = fixes; }
        Err(why) => lint.unchecked = Some(format!("it failed: {why}")),
    }
    lint
}

/// ShellCheck's arguments from the settings, without those Lenz sets (`--shell`, `--format`).
fn user_arguments(value: &Value) -> Vec<String> {
    let words: Vec<String> = match value {
        Value::String(s) => s.split_whitespace().map(str::to_owned).collect(),
        Value::Array(list) => list.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    let mut kept = Vec::new();
    let mut words = words.into_iter();
    while let Some(w) = words.next() {
        if matches!(w.as_str(), "-s" | "--shell" | "-f" | "--format") { words.next(); continue; }
        if w.starts_with("--shell=") || w.starts_with("--format=") || (w.len() > 2 && (w.starts_with("-s") || w.starts_with("-f")) && !w.starts_with("--")) { continue; }
        kept.push(w);
    }
    kept
}

/// Run ShellCheck as `shell` on `doc`'s text: its problems as byte ranges of `doc`, and its fixes.
fn check(exe: &Path, doc: &Document, shell: &str, settings: &Value) -> Result<(Vec<Diagnostic>, Vec<Fix>), String> {
    let dir = doc.source().parent().unwrap_or(Path::new("/"));
    let mut args = vec!["--format=json1".to_owned(), format!("--shell={shell}")];
    if settings.get("shellcheckExternalSources") != Some(&Value::Bool(false)) {
        args.extend(["--external-sources".to_owned(), format!("--source-path={}", dir.display())]);
    }
    args.extend(user_arguments(settings.get("shellcheckArguments").unwrap_or(&Value::Null)));
    args.push("-".to_owned());
    // It runs in the script's folder, where it finds the project's `.shellcheckrc` as for the script itself; 1 means
    // it found problems.
    let out = super::runtime::run_accepting(exe, &args, doc.text(), dir, LINT_TIMEOUT, &[0, 1])?;
    let report: Value = serde_json::from_str(&out).map_err(|_| "it printed something other than its JSON report".to_owned())?;
    Ok(problems(&report, doc))
}

/// ShellCheck's `json1` report about `doc`: its problems and fixes. Lines and columns count from 1, columns in
/// characters (a tab is one); a position it cannot place drops that problem.
pub(crate) fn problems(report: &Value, doc: &Document) -> (Vec<Diagnostic>, Vec<Fix>) {
    let mut items = Vec::new();
    let mut fixes = Vec::new();
    let text = doc.text();
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let header = if text.starts_with("#!") { super::lsp::line_range(doc, 1).map(|r| (r.start, "")).unwrap_or((text.len(), newline)) } else { (0, "") };
    let mut disabled = std::collections::HashSet::new();
    for c in report["comments"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let span = |v: &Value| Some(at(doc, v["line"].as_u64()?, v["column"].as_u64()?)?..at(doc, v["endLine"].as_u64()?, v["endColumn"].as_u64()?)?);
        let Some(range) = span(c).map(|r| r.start..r.end.max(r.start)) else { continue };
        let (Some(message), Some(code)) = (c["message"].as_str(), c["code"].as_u64()) else { continue };
        let severity = match c["level"].as_str() { Some("error") => Severity::Error, Some("warning") => Severity::Warning, Some("info") => Severity::Information, _ => Severity::Hint };
        if let Some(replacements) = c["fix"]["replacements"].as_array().filter(|r| !r.is_empty()) {
            let edits = replacements.iter().map(|r| Some(TextEdit { range: span(r).filter(|s| s.start <= s.end)?, text: r["replacement"].as_str()?.into() })).collect::<Option<Vec<_>>>();
            let edits = edits.ok_or("ShellCheck's fix cannot be placed in this text").and_then(sorted);
            fixes.push(Fix { range: range.clone(), title: format!("Apply fix for SC{code}: {message}"), edits });
        }
        // A directive before the first command applies to the whole file (after the shebang).
        if disabled.insert(code) {
            let (start, prefix) = header;
            fixes.push(Fix { range: range.clone(), title: format!("Disable ShellCheck rule SC{code} for the entire file"), edits: Ok(vec![TextEdit { range: start..start, text: format!("{prefix}# shellcheck disable=SC{code}{newline}").into() }]) });
        } else if let Some(first) = fixes.iter().find(|f| f.title == format!("Disable ShellCheck rule SC{code} for the entire file")).cloned() {
            fixes.push(Fix { range: range.clone(), ..first });
        }
        items.push(Diagnostic { range, severity, message: message.trim().to_owned(), source: Some("shellcheck".into()), code: Some(format!("SC{code}")), raw: c.clone() });
    }
    (items, fixes)
}

/// `edits` in document order, refused when any two overlap or insert at the same place.
fn sorted(mut edits: Vec<TextEdit>) -> Result<Vec<TextEdit>, &'static str> {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    if edits.windows(2).any(|w| w[0].range.end > w[1].range.start || w[0].range == w[1].range) { return Err("ShellCheck's fix has overlapping edits"); }
    Ok(edits)
}

/// The byte offset of ShellCheck's 1-based `line` and character `column` in `doc`, clamped to the line (past the last
/// line: the end of the text).
fn at(doc: &Document, line: u64, column: u64) -> Option<usize> {
    let line = usize::try_from(line.checked_sub(1)?).ok()?;
    let skip = usize::try_from(column.checked_sub(1)?).ok()?;
    let Some(range) = super::lsp::line_range(doc, line) else { return Some(doc.text().len()) };
    let text = doc.text().get(range.clone()).unwrap_or_default();
    Some(range.start + text.char_indices().nth(skip).map_or(text.len(), |(i, _)| i))
}

/// Bash's reserved words that POSIX sh lacks, as bash-language-server 5.8.1 suggests them.
const BASH_WORDS: &[&str] = &["[[", "]]", "function", "select", "time"];
/// Bash's builtins that POSIX sh lacks (`local`, which dash has, apart).
const BASH_BUILTINS: &[&str] = &["bind", "builtin", "caller", "compgen", "complete", "compopt", "declare", "dirs", "disown", "enable", "help", "history", "let", "logout", "mapfile", "popd", "pushd", "readarray", "shopt", "source", "suspend", "typeset"];
/// Bash syntax in a snippet's body: tests, arithmetic, declarations, process substitution, appending, array elements.
const BASH_SYNTAX: &[&str] = &["[[", "((", "declare ", "readarray", "<(", "+=", "]="];

/// Whether bash-language-server's completion `item` (by its `data.type`: 0 builtin, 2 reserved word, 4 snippet) is
/// Bash's own and so not for a `shell` script: sh and dash scripts are not offered Bash's words, builtins or snippets
/// written with Bash syntax. The script's own names and installed commands always stay.
pub fn foreign(shell: Shell, item: &Value) -> bool {
    if !matches!(shell, Shell::Sh | Shell::Dash) { return false; }
    let label = item["label"].as_str().unwrap_or_default();
    match item["data"]["type"].as_u64() {
        Some(0) => BASH_BUILTINS.contains(&label) || (shell == Shell::Sh && label == "local"),
        Some(2) => BASH_WORDS.contains(&label),
        Some(4) => item["insertText"].as_str().is_some_and(|t| BASH_SYNTAX.iter().any(|b| t.contains(b))),
        _ => false,
    }
}

/// What a run of shfmt came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Formatted {
    Text(String),
    /// It cannot be run for this script (missing, off, or not for this shell).
    Unavailable(String),
    Failed(String),
}

/// How long shfmt may take.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(20);

/// shfmt's option flags from bash-language-server's `shfmt` settings, each overridden by the file's EditorConfig key
/// unless `shfmt.ignoreEditorconfig` is set (shfmt itself ignores EditorConfig once a dialect is given, as Lenz gives).
pub(crate) fn shfmt_arguments(dialect: &str, indent: Option<u8>, settings: &Value, editorconfig: &std::collections::BTreeMap<String, (String, PathBuf)>) -> Vec<String> {
    const OPTIONS: [(&str, &str, &str); 6] = [
        ("binaryNextLine", "binary_next_line", "-bn"), ("caseIndent", "switch_case_indent", "-ci"), ("funcNextLine", "function_next_line", "-fn"),
        ("keepPadding", "keep_padding", "-kp"), ("simplifyCode", "simplify", "-s"), ("spaceRedirects", "space_redirects", "-sr"),
    ];
    let own = &settings["shfmt"];
    let ignore = own["ignoreEditorconfig"] == true;
    let mut args: Vec<String> = match &own["additionalArguments"] {
        Value::String(s) => s.split_whitespace().map(str::to_owned).collect(),
        Value::Array(list) => list.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    args.push(format!("-ln={dialect}"));
    args.push(format!("-i={}", indent.unwrap_or(0)));
    for (setting, key, flag) in OPTIONS {
        let from_file = (!ignore).then(|| editorconfig.get(key)).flatten().map(|(v, _)| v == "true");
        if from_file.unwrap_or(own[setting] == true) { args.push(flag.to_owned()); }
    }
    args
}

/// Format `text`, the draft of `file`, with shfmt as its shell; `indent` is the file's indentation (`Some(n)` spaces,
/// `None` tabs). Reads project files and runs shfmt on standard input: off the UI thread.
pub fn format(file: &Path, text: &str, settings: &Value, path: Option<&OsStr>, indent: Option<u8>) -> Formatted {
    let identified = identify(file, head(text), settings.get("shell").and_then(Value::as_str));
    let dialect = match &identified.shell {
        Ok(shell) => match shell.shfmt() { Some(d) => d, None => return Formatted::Unavailable(format!("shfmt does not format {} scripts (the shell is {})", shell.label(), identified.describe())) },
        Err(_) => return Formatted::Unavailable(format!("shfmt formats sh, Bash and ksh scripts; this one is {}", identified.describe())),
    };
    let exe = match shfmt(settings, path) {
        Tool::Found { path, .. } => path,
        tool @ Tool::Missing(_) => return Formatted::Unavailable(format!("{}. {SHFMT_SETUP}", tool.identity("shfmt"))),
        tool @ Tool::Off => return Formatted::Unavailable(tool.identity("shfmt")),
    };
    let args = shfmt_arguments(dialect, indent, settings, &crate::preferences::editorconfig_properties(file));
    let dir = file.parent().unwrap_or(Path::new("/"));
    match super::runtime::run_with_input(&exe, &args, text, dir, FORMAT_TIMEOUT) {
        Ok(out) => Formatted::Text(out),
        Err(why) => Formatted::Failed(format!("shfmt failed as {}: {why}", identified.shell.as_ref().map_or("?", |s| s.label()))),
    }
}

/// bash-language-server's settings as it receives them: ShellCheck and shfmt are Lenz's to run, and Lenz's own `shell`
/// key is not the server's.
pub(super) fn withheld(mut settings: Value) -> Value {
    if let Some(object) = settings.as_object_mut() {
        object.remove("shell");
        object.insert("shellcheckPath".into(), Value::String(String::new()));
        let shfmt = object.entry("shfmt").or_insert_with(|| Value::Object(Default::default()));
        if !shfmt.is_object() { *shfmt = Value::Object(Default::default()); }
        if let Some(shfmt) = shfmt.as_object_mut() { shfmt.insert("path".into(), Value::String(String::new())); }
    }
    settings
}

/// The scripts shown, each checked once per version after edits pause, on one worker thread kept while the session
/// lives; the latest check of each is kept until its tab closes.
#[derive(Default)]
pub(super) struct Linter {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    started: bool,
    asked: HashMap<PathBuf, SourceVersion>,
    results: HashMap<PathBuf, Arc<Lint>>,
}

#[derive(Default)]
struct Queue {
    jobs: HashMap<PathBuf, Job>,
    /// When the latest job came.
    since: Option<Instant>,
    stop: bool,
}

struct Job {
    doc: Arc<Document>,
    settings: Value,
    path: Option<OsString>,
    events: async_channel::Sender<Completion>,
}

impl Linter {
    /// Check `doc` unless this version was asked already; `settings` gives bash-language-server's merged settings and
    /// the `PATH` its tools are looked up on (asked only for a new version).
    pub(super) fn want(&mut self, doc: &Arc<Document>, settings: impl FnOnce() -> (Value, Option<OsString>), events: &async_channel::Sender<Completion>) {
        let source = SourceVersion::new(doc);
        if self.asked.get(doc.source()) == Some(&source) { return; }
        self.asked.insert(doc.source().to_owned(), source);
        let (settings, path) = settings();
        let (lock, wake) = &*self.queue;
        let Ok(mut queue) = lock.lock() else { return };
        queue.jobs.insert(doc.source().to_owned(), Job { doc: doc.clone(), settings, path, events: events.clone() });
        queue.since = Some(Instant::now());
        drop(queue);
        wake.notify_all();
        if !self.started {
            self.started = true;
            let queue = self.queue.clone();
            if std::thread::Builder::new().name("lenz-shellcheck".into()).spawn(move || work(&queue)).is_err() { self.started = false; }
        }
    }

    /// Forget the scripts no tab shows any more (`shown`: every file tab's source; a few, so searched in place).
    pub(super) fn retain(&mut self, shown: &[&Path]) {
        if self.asked.is_empty() && self.results.is_empty() { return; }
        let kept = |p: &PathBuf| shown.contains(&p.as_path());
        self.asked.retain(|p, _| kept(p));
        self.results.retain(|p, _| kept(p));
        if let Ok(mut queue) = self.queue.0.lock() { queue.jobs.retain(|p, _| kept(p)); }
    }

    /// Check every script again on its next synchronization (settings changed).
    pub(super) fn again(&mut self) { self.asked.clear(); }

    /// Whether Lenz checks `path` itself (its problems are ShellCheck's, not a server's).
    pub(super) fn checks(&self, path: &Path) -> bool { self.asked.contains_key(path) || self.results.contains_key(path) }

    pub(super) fn latest(&self, path: &Path) -> Option<&Arc<Lint>> { self.results.get(path) }

    /// A check finished: kept while its script is shown.
    pub(super) fn finished(&mut self, path: PathBuf, lint: Arc<Lint>) -> bool {
        if !self.asked.contains_key(&path) { return false; }
        self.results.insert(path, lint);
        true
    }
}

impl Drop for Linter {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.queue.0.lock() { queue.stop = true; queue.jobs.clear(); }
        self.queue.1.notify_all();
    }
}

/// The worker: once edits have paused, check every script waiting (the latest version of each), and send each result.
fn work(queue: &(Mutex<Queue>, Condvar)) {
    let (lock, wake) = queue;
    loop {
        let jobs = {
            let Ok(mut q) = lock.lock() else { return };
            loop {
                if q.stop { return; }
                match q.since {
                    _ if q.jobs.is_empty() => { let Ok(next) = wake.wait(q) else { return }; q = next; }
                    Some(since) if since.elapsed() < PAUSE => { let Ok((next, _)) = wake.wait_timeout(q, PAUSE - since.elapsed()) else { return }; q = next; }
                    _ => break,
                }
            }
            std::mem::take(&mut q.jobs)
        };
        for (path, job) in jobs {
            let lint = lint(&job.doc, &job.settings, job.path.as_deref());
            let _ = job.events.send_blocking(Completion::Semantic(super::Event::Linted { path, lint: Arc::new(lint) }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(path: &str, text: &str) -> Document { Document::decode(PathBuf::from(path), 1, text.as_bytes()) }

    fn shell(head: &str, file: &str, setting: Option<&str>, rc: Option<&str>, variant: Option<&str>) -> (Result<Shell, String>, Basis) {
        let i = identify_in(head, Path::new(file), setting, rc.map(|v| (v, Path::new("/p/.shellcheckrc"))), variant.map(|v| (v, Path::new("/p/.editorconfig"))));
        (i.shell, i.basis)
    }

    #[test]
    fn the_shell_comes_from_the_directive_then_the_shebang_name_setting_shellcheckrc_and_editorconfig() {
        use Shell::*;
        let all = (Some("ksh"), Some("dash"), Some("mksh"));
        let every = |head: &str, file: &str| shell(head, file, all.0, all.1, all.2);
        // The file's own directive wins over its shebang (as ShellCheck reads it), then the shebang, …
        assert_eq!(every("#!/bin/bash\n# Build.\n\n# shellcheck disable=SC2086 shell=sh\necho\n", "/p/a.bash"), (Ok(Sh), Basis::Directive));
        assert_eq!(every("#!/usr/bin/env -S bash -e\necho\n", "/p/a.sh"), (Ok(Bash), Basis::Shebang("#!/usr/bin/env -S bash -e".into())));
        assert_eq!(every("#!/bin/sh -e\n", "/p/a.bash").0, Ok(Sh));
        assert_eq!(every("#! /bin/dash\n", "/p/a").0, Ok(Dash));
        assert_eq!(every("#!/usr/bin/ksh93\n", "/p/a").0, Ok(Ksh));
        assert_eq!(every("#!/bin/busybox sh\n", "/p/a").0, Ok(Dash));
        // A directive after the first command is not file-wide; a shebang naming another program is not a shell.
        assert_eq!(every("#!/bin/sh\necho\n# shellcheck shell=bash\n", "/p/a").0, Ok(Sh));
        assert_eq!(every("#!/usr/bin/env python3\n", "/p/a.sh"), (Err("python3".into()), Basis::Shebang("#!/usr/bin/env python3".into())));
        // … then a name naming one shell (`.sh` names none), the setting, .shellcheckrc, EditorConfig, else Bash.
        assert_eq!(every("echo\n", "/p/a.bash"), (Ok(Bash), Basis::Name(".bash".into())));
        assert_eq!(every("echo\n", "/p/.profile"), (Ok(Sh), Basis::Name(".profile".into())));
        assert_eq!(every("echo\n", "/p/.zshrc").0, Ok(Zsh));
        assert_eq!(every("echo\n", "/p/a.sh"), (Ok(Ksh), Basis::Setting));
        assert_eq!(shell("echo\n", "/p/a.sh", None, Some("dash"), Some("mksh")), (Ok(Dash), Basis::ShellcheckRc("/p/.shellcheckrc".into())));
        assert_eq!(shell("echo\n", "/p/a.sh", Some(" "), None, Some("posix")), (Ok(Sh), Basis::EditorConfig("/p/.editorconfig".into())));
        assert_eq!(shell("echo\n", "/p/a.sh", None, None, Some("auto")), (Ok(Bash), Basis::Default));
        assert_eq!(shell("echo\n", "/p/a.sh", Some("fish"), None, None), (Err("fish".into()), Basis::Setting));
        assert_eq!(shell("", "/p/run", None, None, None), (Ok(Bash), Basis::Default));
    }

    #[test]
    fn identification_reads_the_nearest_shellcheckrc_only_when_the_script_names_no_shell() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("project");
        std::fs::create_dir_all(root.join("lib/deep")).unwrap();
        std::fs::write(root.join(".shellcheckrc"), "# Portable scripts.\nshell=sh # here\n").unwrap();
        std::fs::write(root.join("lib/.shellcheckrc"), "disable=SC2086\n").unwrap();
        let top = identify(&root.join("run.sh"), "echo\n", None);
        assert_eq!((top.shell, top.basis), (Ok(Shell::Sh), Basis::ShellcheckRc(root.join(".shellcheckrc"))));
        // The nearest file is ShellCheck's: one that names no shell leaves the default.
        assert_eq!(identify(&root.join("lib/deep/x.sh"), "echo\n", None).basis, Basis::Default);
        assert_eq!(identify(&root.join("run.sh"), "#!/bin/bash\n", None).shell, Ok(Shell::Bash));
        std::fs::write(root.join("lib/.editorconfig"), "[*.sh]\nshell_variant = mksh\n").unwrap();
        assert_eq!(identify(&root.join("lib/deep/x.sh"), "echo\n", None).shell, Ok(Shell::Ksh));
    }

    #[test]
    fn shellchecks_character_columns_are_placed_exactly_after_tabs_and_characters_outside_the_bmp() {
        // `$foo` follows `😀é` (two UTF-16 units, then one) and, on the next line, two tabs.
        let text = "#!/bin/sh\necho \"😀é\" $foo\n\t\techo $x\n";
        let d = doc("/p/a.sh", text);
        let report = serde_json::json!({ "comments": [
            { "line": 2, "endLine": 2, "column": 11, "endColumn": 15, "level": "info", "code": 2086, "message": "Double quote to prevent globbing and word splitting.",
              "fix": { "replacements": [
                { "line": 2, "endLine": 2, "column": 11, "endColumn": 11, "insertionPoint": "afterEnd", "precedence": 7, "replacement": "\"" },
                { "line": 2, "endLine": 2, "column": 15, "endColumn": 15, "insertionPoint": "beforeStart", "precedence": 7, "replacement": "\"" } ] } },
            { "line": 3, "endLine": 3, "column": 8, "endColumn": 10, "level": "warning", "code": 2154, "message": "x is referenced but not assigned.", "fix": null },
            { "line": 9, "endLine": 9, "column": 1, "endColumn": 2, "level": "style", "code": 1, "message": "past the end", "fix": null },
        ] });
        let (items, fixes) = problems(&report, &d);
        assert_eq!(&text[items[0].range.clone()], "$foo");
        assert_eq!(&text[items[1].range.clone()], "$x");
        assert_eq!((items[0].severity, items[1].severity, items[2].severity), (Severity::Information, Severity::Warning, Severity::Hint));
        assert_eq!((items[0].code.as_deref(), items[0].source.as_deref()), (Some("SC2086"), Some("shellcheck")));
        assert_eq!(items[2].range, text.len()..text.len(), "a position past the text is its end");
        let quote = &fixes[0];
        assert_eq!(quote.title, "Apply fix for SC2086: Double quote to prevent globbing and word splitting.");
        let mut fixed = text.to_owned();
        for e in quote.edits.as_ref().unwrap().iter().rev() { fixed.replace_range(e.range.clone(), &e.text); }
        assert_eq!(fixed, "#!/bin/sh\necho \"😀é\" \"$foo\"\n\t\techo $x\n");
        // Every rule can be disabled for the file, after the shebang, once per rule.
        assert_eq!(fixes[1].title, "Disable ShellCheck rule SC2086 for the entire file");
        let insert = &fixes[1].edits.as_ref().unwrap()[0];
        assert_eq!((insert.range.clone(), &*insert.text), (10..10, "# shellcheck disable=SC2086\n"));
        assert_eq!(fixes.iter().filter(|f| f.title.starts_with("Disable")).count(), 3);
    }

    #[test]
    fn a_disable_directive_goes_first_without_a_shebang_and_on_a_new_line_after_a_lone_one() {
        let report = serde_json::json!({ "comments": [{ "line": 1, "endLine": 1, "column": 1, "endColumn": 2, "level": "error", "code": 2148, "message": "m", "fix": null }] });
        let (_, fixes) = problems(&report, &doc("/p/a", "echo\r\n"));
        assert_eq!(&*fixes[0].edits.as_ref().unwrap()[0].text, "# shellcheck disable=SC2148\r\n");
        assert_eq!(fixes[0].edits.as_ref().unwrap()[0].range, 0..0);
        let (_, fixes) = problems(&report, &doc("/p/a", "#!/bin/sh"));
        let edit = &fixes[0].edits.as_ref().unwrap()[0];
        assert_eq!((edit.range.clone(), &*edit.text), (9..9, "\n# shellcheck disable=SC2148\n"));
    }

    #[test]
    fn shellcheck_and_shfmt_arguments_keep_lenzs_shell_and_follow_editorconfig_over_settings() {
        let settings = serde_json::json!({ "shellcheckArguments": "--shell=bash -e SC1091 -s zsh --format gcc -x", "shfmt": { "caseIndent": false, "spaceRedirects": true, "binaryNextLine": true } });
        assert_eq!(user_arguments(&settings["shellcheckArguments"]), ["-e", "SC1091", "-x"]);
        assert_eq!(user_arguments(&serde_json::json!(["-sbash", " -o all "])), ["-o all"]);
        let mut editorconfig = std::collections::BTreeMap::new();
        editorconfig.insert("switch_case_indent".to_owned(), ("true".to_owned(), PathBuf::from("/p/.editorconfig")));
        editorconfig.insert("binary_next_line".to_owned(), ("false".to_owned(), PathBuf::from("/p/.editorconfig")));
        assert_eq!(shfmt_arguments("posix", Some(3), &settings, &editorconfig), ["-ln=posix", "-i=3", "-ci", "-sr"]);
        let mut ignore = settings.clone();
        ignore["shfmt"]["ignoreEditorconfig"] = true.into();
        ignore["shfmt"]["additionalArguments"] = "-mn".into();
        assert_eq!(shfmt_arguments("bash", None, &ignore, &editorconfig), ["-mn", "-ln=bash", "-i=0", "-bn", "-sr"]);
    }

    #[test]
    fn the_server_never_receives_the_tools_lenz_runs_or_lenzs_shell_setting() {
        let sent = withheld(serde_json::json!({ "shell": "sh", "shellcheckPath": "/opt/sc", "shfmt": { "path": "/opt/shfmt", "caseIndent": true }, "globPattern": "**/*.sh" }));
        assert_eq!(sent, serde_json::json!({ "shellcheckPath": "", "shfmt": { "path": "", "caseIndent": true }, "globPattern": "**/*.sh" }));
        assert_eq!(withheld(serde_json::json!({})), serde_json::json!({ "shellcheckPath": "", "shfmt": { "path": "" } }));
    }

    #[test]
    fn an_sh_script_is_not_offered_bashs_own_words_builtins_or_snippets() {
        let item = |label: &str, kind: u8, text: Option<&str>| serde_json::json!({ "label": label, "data": { "type": kind }, "insertText": text });
        let (keyword, builtin, executable, symbol, snippet) = (2, 0, 1, 3, 4);
        for (shell, offered) in [(Shell::Sh, false), (Shell::Dash, false), (Shell::Bash, true), (Shell::Ksh, true)] {
            assert_eq!(!foreign(shell, &item("[[", keyword, None)), offered, "{shell:?}");
            assert_eq!(!foreign(shell, &item("declare", builtin, None)), offered);
            assert_eq!(!foreign(shell, &item("if-string-empty", snippet, Some("if [[ -z \"${1:expression}\" ]]; then\n\t${2:command ...}\nfi"))), offered);
            assert_eq!(!foreign(shell, &item("variable-append", snippet, Some("${1:variable}+=${2:value}"))), offered);
            // POSIX words, the script's own names and installed commands stay.
            for kept in [item("if", keyword, None), item("export", builtin, None), item("declare", symbol, None), item("declare", executable, None), item("if", snippet, Some("if ${1:condition}; then\n\t${2:command ...}\nfi"))] {
                assert!(!foreign(shell, &kept), "{kept}");
            }
        }
        // `local` is dash's (and ShellCheck accepts it there), not POSIX sh's.
        assert!(foreign(Shell::Sh, &item("local", builtin, None)) && !foreign(Shell::Dash, &item("local", builtin, None)));
    }

    #[test]
    fn tools_are_named_off_or_missing_as_their_settings_say() {
        let none = Some(OsStr::new("/nonexistent"));
        assert_eq!(shellcheck(&serde_json::json!({}), none), Tool::Missing("shellcheck".into()));
        assert_eq!(shfmt(&serde_json::json!({ "shfmt": { "path": "" } }), none), Tool::Off);
        assert_eq!(shellcheck(&serde_json::json!({ "shellcheckPath": "/nonexistent/sc" }), none).identity("ShellCheck"), "ShellCheck is missing (/nonexistent/sc does not exist)");
        assert_eq!(shfmt(&serde_json::json!({}), none).identity("shfmt"), "shfmt is missing (`shfmt` is not on PATH)");
    }
}
