//! ERB templates' configured formatting (ticket 58). Ruby LSP formats no template (RuboCop skips `.erb` files), and
//! the HTML server's formatting would rewrite the template's own tags, so Format Document and format-on-save run the
//! project's own erb_lint, when an `.erb_lint.yml` configures it, with `--autocorrect` on the draft through standard
//! input (`--stdin`, which reads the configuration's rules for the template's own path), off the UI thread, fenced to
//! the version given as any project formatter. erb_lint runs as the project runs it: its binstub, else `bundle exec`
//! when the project's bundle locks it, else the one on `PATH` (the Ruby provider's). It loads the project's
//! configuration (and the RuboCop configuration and custom linters that names), as on the command line; it writes
//! nothing when it reads standard input.
//!
//! erb_lint 0.9.0 prints its report, then a `================ <path> ==================` line, then the corrected
//! text (with a newline added when it lacks one); a template it cannot process is reported as an exception and its
//! text printed empty, which Lenz refuses. Its corrections go through parser's source buffer, which reads CRLF line
//! ends as LF: Lenz puts a CRLF file's line ends back (a file mixing both is refused).
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use super::provider::find_executable;

/// The files configuring erb_lint, in its order (`.erb-lint.yml` is its older name, still read).
pub const CONFIGS: [&str; 2] = [".erb_lint.yml", ".erb-lint.yml"];

/// How long erb_lint may take (it loads RuboCop, and a bundle).
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How the project's erb_lint is run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// The project's binstub (`bin/erb_lint`), which sets up its bundle itself.
    Binstub(PathBuf),
    /// `bundle exec erb_lint` in the bundle of `gemfile`, which locks erb_lint.
    Bundle { bundle: PathBuf, gemfile: PathBuf },
    /// The erb_lint on `PATH`, outside the project's bundle.
    Path(PathBuf),
}

/// The erb_lint a template's project configures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErbLint {
    pub config: PathBuf,
    /// `None`: not installed where the project would run it (Lenz never installs it).
    pub run: Option<Run>,
}

impl ErbLint {
    /// What Server Details and the editing preferences say about it.
    pub fn describe(&self) -> String {
        let configured = format!("erb_lint (configured by {})", self.config.display());
        match &self.run {
            Some(Run::Binstub(b)) => format!("{configured}; run as {} with --autocorrect", b.display()),
            Some(Run::Bundle { gemfile, .. }) => format!("{configured}; run with --autocorrect by `bundle exec` in the bundle of {}", gemfile.display()),
            Some(Run::Path(p)) => format!("{configured}; executable {}, run with --autocorrect outside the project's bundle", p.display()),
            None => format!("{configured}; not found as bin/erb_lint, in the project's bundle or on PATH. Lenz never installs formatters"),
        }
    }
}

/// What Format Document says when no erb_lint is configured.
pub const NOT_CONFIGURED: &str = "no ERB formatter is configured for this project (an .erb_lint.yml for erb_lint); Ruby LSP does not format templates, and Lenz never installs formatters";

/// The erb_lint `file`'s project configures: the nearest configuration at or above its folder within `root`, with
/// how the project runs it (`path`: the `PATH` to look on). Reads project files; never runs anything.
pub fn configured(file: &Path, root: &Path, path: Option<&OsStr>) -> Option<ErbLint> {
    let dirs: Vec<&Path> = file.ancestors().skip(1).take_while(|d| d.starts_with(root)).collect();
    let config = dirs.iter().find_map(|d| CONFIGS.iter().map(|c| d.join(c)).find(|c| c.is_file()))?;
    let binstub = dirs.iter().flat_map(|d| ["bin/erb_lint", "bin/erblint"].map(|b| d.join(b))).find(|b| runnable(b)).map(Run::Binstub);
    let bundle = || {
        let gemfile = dirs.iter().find_map(|d| ["Gemfile", "gems.rb"].iter().map(|g| d.join(g)).find(|g| g.is_file()))?;
        let lock = gemfile.with_file_name(if gemfile.ends_with("gems.rb") { "gems.locked" } else { "Gemfile.lock" });
        super::provider::locks(&std::fs::read_to_string(lock).ok()?, "erb_lint").then_some(())?;
        Some(Run::Bundle { bundle: find_executable(Path::new("bundle"), path)?, gemfile })
    };
    let on_path = || ["erb_lint", "erblint"].iter().find_map(|n| find_executable(Path::new(n), path)).map(Run::Path);
    Some(ErbLint { config, run: binstub.or_else(bundle).or_else(on_path) })
}

/// What a run came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Formatted {
    /// The whole corrected text.
    Text(String),
    /// It is not configured, not installed, or cannot read this template: nothing ran.
    Unavailable(String),
    /// It ran and failed: nothing changes.
    Failed(String),
}

/// Run `file`'s project erb_lint with `--autocorrect` on `text` (see the module). Runs a tool: call off the UI thread.
pub fn format(file: &Path, text: &str, root: &Path, path: Option<&OsStr>) -> Formatted {
    let Some(lint) = configured(file, root, path) else { return Formatted::Unavailable(NOT_CONFIGURED.into()) };
    let Some(run) = lint.run.clone() else { return Formatted::Unavailable(lint.describe()) };
    // erb_lint reads a template's rules by its path, which must exist.
    if !file.is_file() { return Formatted::Unavailable("erb_lint finds a template's rules by its file, which is not saved yet: save it first".into()); }
    let cwd = lint.config.parent().unwrap_or(root);
    let own = ["--config".to_owned(), lint.config.display().to_string(), "--stdin".into(), file.display().to_string(), "--autocorrect".into()];
    let mut env: Vec<(String, String)> = path.map(|p| ("PATH".to_owned(), p.to_string_lossy().into_owned())).into_iter().collect();
    let (exe, args) = match run {
        Run::Binstub(b) | Run::Path(b) => (b, own.to_vec()),
        Run::Bundle { bundle, gemfile } => {
            env.push(("BUNDLE_GEMFILE".into(), gemfile.display().to_string()));
            (bundle, ["exec".to_owned(), "erb_lint".into()].into_iter().chain(own).collect())
        }
    };
    // It exits 1 when problems it cannot correct remain: its text is printed all the same.
    match super::runtime::run_with_input_env(&exe, &args, text, cwd, TIMEOUT, &[0, 1], &env) {
        Ok(out) => corrected(&out, text),
        Err(why) => Formatted::Failed(format!("erb_lint failed: {why}")),
    }
}

/// The corrected text in erb_lint's output for `text` (see the module).
fn corrected(out: &str, text: &str) -> Formatted {
    if let Some(at) = out.find("Exception occurred when processing:") {
        // The exception's message is the third line of the report.
        let message = out.get(at..).unwrap_or_default().lines().nth(2).unwrap_or_default().trim();
        return Formatted::Failed(format!("erb_lint could not process this template ({message}); nothing changed"));
    }
    let mut start = None;
    let mut offset = 0;
    for line in out.split_inclusive('\n') {
        offset += line.len();
        let bare = line.trim_end_matches('\n');
        if bare.starts_with("================ ") && bare.ends_with(" ==================") { start = Some(offset); }
    }
    let Some(start) = start else { return Formatted::Failed("erb_lint printed no corrected text; nothing changed".into()) };
    let found = out.get(start..).unwrap_or_default();
    let found = if text.contains("\r\n") && !found.contains("\r\n") && found.contains('\n') {
        let bytes = text.as_bytes();
        let mixed = bytes.iter().enumerate().any(|(i, b)| *b == b'\n' && (i == 0 || bytes.get(i - 1) != Some(&b'\r')));
        if mixed { return Formatted::Failed("erb_lint rewrote this file's mixed CRLF and LF line ends as LF; nothing changed".into()); }
        found.replace('\n', "\r\n")
    } else { found.to_owned() };
    Formatted::Text(found)
}

fn runnable(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else { return false };
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; meta.is_file() && meta.permissions().mode() & 0o111 != 0 }
    #[cfg(not(unix))]
    { meta.is_file() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p
    }

    const HEADER: &str = "================ /p/app/views/a.html.erb ==================\n";

    #[test]
    fn the_corrected_text_follows_its_header_and_crlf_line_ends_are_put_back() {
        let report = "Linting and autocorrecting 1 files with 14 linters (11 autocorrectable)...\n\n2 error(s) corrected in ERB files\n";
        assert_eq!(corrected(&format!("{report}{HEADER}<p><%= x %></p>\n"), "<p><%=x%></p>\n"), Formatted::Text("<p><%= x %></p>\n".into()));
        // A CRLF file: its own line ends, also for text erb_lint left as it was.
        assert_eq!(corrected(&format!("{report}{HEADER}<p>\n<%= é %>\n</p>\n"), "<p>\r\n<%=é%>\r\n</p>\r\n"), Formatted::Text("<p>\r\n<%= é %>\r\n</p>\r\n".into()));
        assert_eq!(corrected(&format!("{report}{HEADER}<p>\r\n</p>\r\n"), "<p>\r\n</p>\r\n"), Formatted::Text("<p>\r\n</p>\r\n".into()));
        assert!(matches!(corrected(&format!("{HEADER}a\nb\nc\n"), "a\r\nb\nc\r\n"), Formatted::Failed(w) if w.contains("mixed")));
        // A template it cannot process: its exception, and its empty text refused.
        let failed = "Linting and autocorrecting 1 files with 14 linters (11 autocorrectable)...\nException occurred when processing: a.html.erb\nIf this file cannot be processed by erb_lint, you can exclude it in your configuration file.\nbegin_pos location 44 is out of range for document of size 43\n/gems/location.rb:14:in `initialize'\n\n\nNo errors were found in ERB files\n";
        assert_eq!(corrected(&format!("{failed}{HEADER}\n"), "x"), Formatted::Failed("erb_lint could not process this template (begin_pos location 44 is out of range for document of size 43); nothing changed".into()));
        assert!(matches!(corrected("erb_lint: no linter available\n", "x"), Formatted::Failed(_)));
    }

    #[test]
    fn erb_lint_is_found_by_its_configuration_and_run_as_the_project_runs_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app");
        let file = write(&root, "app/views/posts/show.html.erb", "");
        assert_eq!(configured(&file, &root, Some(OsStr::new(""))), None, "not configured");
        let config = write(&root, ".erb-lint.yml", "---\n");
        let bin = dir.path().join("bin");
        let tool = write(&bin, "erb_lint", "#!/bin/sh\n");
        let bundle = write(&bin, "bundle", "#!/bin/sh\n");
        for f in [&tool, &bundle] { std::fs::set_permissions(f, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap(); }
        let path = bin.as_os_str();
        assert_eq!(configured(&file, &root, Some(OsStr::new(""))), Some(ErbLint { config: config.clone(), run: None }), "configured, not installed");
        assert_eq!(configured(&file, &root, Some(path)), Some(ErbLint { config: config.clone(), run: Some(Run::Path(tool.clone())) }));
        // Its current name wins over the older one; a bundle locking it runs it.
        let config = write(&root, ".erb_lint.yml", "---\n");
        let gemfile = write(&root, "Gemfile", "gem \"erb_lint\"\n");
        write(&root, "Gemfile.lock", "GEM\n  specs:\n    erb_lint (0.9.0)\n\nDEPENDENCIES\n  erb_lint\n");
        assert_eq!(configured(&file, &root, Some(path)), Some(ErbLint { config: config.clone(), run: Some(Run::Bundle { bundle, gemfile }) }));
        let stub = write(&root, "bin/erb_lint", "#!/bin/sh\n");
        std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert_eq!(configured(&file, &root, Some(path)).unwrap().run, Some(Run::Binstub(stub)));
        // Outside the root: not looked for.
        assert_eq!(configured(&file, &root.join("app"), Some(path)), None);
    }
}
