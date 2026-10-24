//! Editing preferences for one file: EditorConfig over the project's own choice over the language default.
//! Resolution only reads `.editorconfig` files; it never writes, installs or runs anything.
use std::path::{Path, PathBuf};

pub use lenz_content::highlight::Language;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style { Space, Tab }

/// A project's own indentation choice, remembered by Lenz outside the checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Indent { Tabs, Spaces(u8) }

/// What Lenz remembers for one project, outside its checkout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProjectPreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent: Option<Indent>,
    /// Off by default: formatting is explicit.
    pub format_on_save: bool,
}

/// Where a resolved value came from, highest precedence first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin { EditorConfig(PathBuf), Project, Default }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub style: (Style, Origin),
    pub size: (u8, Origin),
    pub tab_width: (u8, Origin),
}

/// The raw EditorConfig properties for one file, each with the file that set it.
#[derive(Default)]
struct Found {
    style: Option<(Style, PathBuf)>,
    /// `None` inside: `indent_size = tab`.
    size: Option<(Option<u8>, PathBuf)>,
    tab_width: Option<(u8, PathBuf)>,
}

/// `file`'s indentation: EditorConfig, then `project`, then the language's convention.
pub fn resolve(file: &Path, project: Option<Indent>, language: Option<Language>) -> Resolved {
    let found = editorconfig(file);
    let style = match (found.style, project) {
        (Some((s, from)), _) => (s, Origin::EditorConfig(from)),
        (None, Some(Indent::Tabs)) => (Style::Tab, Origin::Project),
        (None, Some(Indent::Spaces(_))) => (Style::Space, Origin::Project),
        (None, None) => (if matches!(language, Some(Language::Go | Language::Makefile)) { Style::Tab } else { Style::Space }, Origin::Default),
    };
    let tab = found.tab_width.clone().map(|(n, from)| (n, Origin::EditorConfig(from)));
    let size = match (found.size, project) {
        (Some((Some(n), from)), _) => (n, Origin::EditorConfig(from)),
        (Some((None, _)), _) if tab.is_some() => tab.clone().unwrap_or((4, Origin::Default)),
        (_, Some(Indent::Spaces(n))) => (n, Origin::Project),
        _ => (4, Origin::Default),
    };
    let tab_width = tab.unwrap_or_else(|| size.clone());
    Resolved { style, size, tab_width }
}

/// EditorConfig files are small; a larger one is ignored rather than read whole.
const MAX_EDITORCONFIG: u64 = 64 * 1024;

fn editorconfig(file: &Path) -> Found {
    let mut chain = Vec::new();
    for dir in file.ancestors().skip(1) {
        let path = dir.join(".editorconfig");
        let Some(text) = std::fs::metadata(&path).ok().filter(|m| m.is_file() && m.len() <= MAX_EDITORCONFIG).and_then(|_| std::fs::read_to_string(&path).ok()) else { continue };
        let root = parse(&text).0;
        chain.push((dir.to_owned(), path, text));
        if root { break; }
    }
    let mut found = Found::default();
    // Outermost first, so nearer files and later sections override.
    for (dir, path, text) in chain.into_iter().rev() {
        let Ok(relative) = file.strip_prefix(&dir) else { continue };
        for (pattern, props) in parse(&text).1 {
            if !matches(&pattern, relative) { continue; }
            for (key, value) in props {
                let from = path.clone();
                match (key.as_str(), value.as_str()) {
                    ("indent_style", "unset") => found.style = None,
                    ("indent_style", "space") => found.style = Some((Style::Space, from)),
                    ("indent_style", "tab") => found.style = Some((Style::Tab, from)),
                    ("indent_size", "unset") => found.size = None,
                    ("indent_size", "tab") => found.size = Some((None, from)),
                    ("indent_size", n) => if let Some(n) = width(n) { found.size = Some((Some(n), from)) },
                    ("tab_width", "unset") => found.tab_width = None,
                    ("tab_width", n) => if let Some(n) = width(n) { found.tab_width = Some((n, from)) },
                    _ => {}
                }
            }
        }
    }
    found
}

fn width(value: &str) -> Option<u8> { value.parse().ok().filter(|n| (1..=16).contains(n)) }

type Section = (String, Vec<(String, String)>);

/// `root = true` before any section, and each section's lowercased properties.
fn parse(text: &str) -> (bool, Vec<Section>) {
    let (mut root, mut sections) = (false, Vec::<Section>::new());
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) { continue; }
        if let Some(pattern) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((pattern.to_owned(), Vec::new()));
        } else if let Some((key, value)) = line.split_once(['=', ':']) {
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim().to_ascii_lowercase());
            match sections.last_mut() {
                Some((_, props)) => props.push((key, value)),
                None => root |= key == "root" && value == "true",
            }
        }
    }
    (root, sections)
}

/// EditorConfig globbing through globset. ponytail: `{n..m}` numeric ranges never match; add them if a project needs one.
fn matches(pattern: &str, relative: &Path) -> bool {
    let pattern = match pattern.strip_prefix('/') {
        Some(anchored) => anchored.to_owned(),
        None if pattern.contains('/') => pattern.to_owned(),
        None => format!("**/{pattern}"),
    };
    // `**` inside a segment (`**.md`) crosses directories in EditorConfig; globset wants it alone.
    let pattern = pattern.replace("**.", "**/*.");
    globset::GlobBuilder::new(&pattern).literal_separator(true).build().is_ok_and(|g| g.compile_matcher().is_match(relative))
}

/// The formatter the formatting slice would run for a file, found by looking at files only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formatter {
    pub tool: &'static str,
    /// The project file that configures it (`None`: the tool's own defaults).
    pub config: Option<PathBuf>,
    /// A project-provided executable, else one on `PATH`; `None`: not installed (Lenz never installs it).
    pub executable: Option<PathBuf>,
}

/// A configuration marker: a file name, and optionally text the file must contain.
type Marker = (&'static str, Option<&'static str>);

/// Per language: (tool, whether it applies without project configuration, markers, project-local executables).
fn formatters(language: Language) -> &'static [(&'static str, bool, &'static [Marker], &'static [&'static str])] {
    const PRETTIER: &[Marker] = &[(".prettierrc", None), (".prettierrc.json", None), (".prettierrc.yaml", None), (".prettierrc.yml", None), (".prettierrc.toml", None), (".prettierrc.js", None), (".prettierrc.cjs", None), (".prettierrc.mjs", None), ("prettier.config.js", None), ("prettier.config.cjs", None), ("prettier.config.mjs", None), ("package.json", Some("\"prettier\""))];
    match language {
        Language::Rust => &[("rustfmt", true, &[("rustfmt.toml", None), (".rustfmt.toml", None)], &[])],
        Language::Go => &[("gofmt", true, &[], &[])],
        Language::Bash => &[("shfmt", true, &[], &[])],
        Language::Python => &[
            ("ruff", false, &[("ruff.toml", None), (".ruff.toml", None), ("pyproject.toml", Some("[tool.ruff"))], &[".venv/bin/ruff", "venv/bin/ruff"]),
            ("black", false, &[("pyproject.toml", Some("[tool.black"))], &[".venv/bin/black", "venv/bin/black"]),
        ],
        Language::Ruby => &[("rubocop", false, &[(".rubocop.yml", None)], &["bin/rubocop"])],
        Language::JavaScript | Language::TypeScript | Language::Tsx | Language::Html | Language::Css | Language::Json | Language::Yaml | Language::Markdown => {
            &[("prettier", false, PRETTIER, &["node_modules/.bin/prettier"])]
        }
        _ => &[],
    }
}

/// `file`'s formatter inside the project at `root`; `path` is the `PATH` to search. Reads bounded project files and
/// directory entries; never runs a tool, a shell or an environment file.
pub fn formatter(file: &Path, root: &Path, language: Option<Language>, path: Option<&std::ffi::OsStr>) -> Option<Formatter> {
    if !file.starts_with(root) { return None; }
    let dirs: Vec<&Path> = file.ancestors().skip(1).take_while(|d| d.starts_with(root)).collect();
    formatters(language?).iter().find_map(|&(tool, standard, markers, local)| {
        let config = dirs.iter().find_map(|d| markers.iter().map(|(name, needle)| (d.join(name), needle)).find(|(f, needle)| {
            needle.map_or_else(|| f.is_file(), |n| lenz_content::theme::read_bounded(f).is_ok_and(|b| String::from_utf8_lossy(&b).contains(n)))
        }).map(|(f, _)| f));
        (standard || config.is_some()).then(|| {
            let local = dirs.iter().flat_map(|d| local.iter().map(move |l| d.join(l)));
            let on_path = path.into_iter().flat_map(std::env::split_paths).map(|d| d.join(tool));
            Formatter { tool, config, executable: local.chain(on_path).find(|f| runnable(f)) }
        })
    })
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

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn nearest_editorconfig_wins_until_root_and_each_value_names_its_source() {
        let scratch = tempfile::tempdir().unwrap();
        let outer = scratch.path();
        let root = outer.join("project");
        write(outer, ".editorconfig", "[*]\nindent_style = tab\n");
        write(&root, ".editorconfig", "root = true\n; comment\n[*]\nindent_style = space\nindent_size = 2\n\n[*.{py,pyi}]\nINDENT_SIZE = 4\n\n[/docs/**.md]\nindent_style = tab\ntab_width = 8\n");
        write(&root, "sub/.editorconfig", "[b.py]\nindent_size = unset\n[Makefile]\nindent_style = tab\nindent_size = tab\ntab_width = 8\n");
        let top = root.join(".editorconfig");
        let sub = root.join("sub/.editorconfig");

        let rs = resolve(&root.join("src/a.rs"), None, Some(Language::Rust));
        assert_eq!(rs.style, (Style::Space, Origin::EditorConfig(top.clone())));
        assert_eq!(rs.size, (2, Origin::EditorConfig(top.clone())));
        assert_eq!(rs.tab_width, (2, Origin::EditorConfig(top.clone())));

        let py = resolve(&root.join("lib/c.py"), None, Some(Language::Python));
        assert_eq!(py.size, (4, Origin::EditorConfig(top.clone())));
        // `unset` in the nearer file returns the size to the next source, not to the outer file's value.
        let unset = resolve(&root.join("sub/b.py"), Some(Indent::Spaces(3)), Some(Language::Python));
        assert_eq!(unset.size, (3, Origin::Project));
        assert_eq!(unset.style, (Style::Space, Origin::EditorConfig(top.clone())));

        let make = resolve(&root.join("sub/Makefile"), None, Some(Language::Makefile));
        assert_eq!(make.style, (Style::Tab, Origin::EditorConfig(sub.clone())));
        assert_eq!(make.size, (8, Origin::EditorConfig(sub.clone())), "indent_size = tab uses tab_width");

        let md = resolve(&root.join("docs/x/y.md"), None, Some(Language::Markdown));
        assert_eq!(md.style, (Style::Tab, Origin::EditorConfig(top.clone())));
        assert_eq!(md.tab_width, (8, Origin::EditorConfig(top.clone())));
        // Anchored to the directory of the `.editorconfig` that declares it.
        assert_eq!(resolve(&root.join("sub/docs/z.md"), None, None).style.0, Style::Space);
    }

    #[test]
    fn without_editorconfig_the_project_choice_then_the_language_default_applies() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("p");
        std::fs::create_dir(&root).unwrap();
        // Shell syntax in an environment file is data: nothing reads or runs it.
        write(&root, ".env", "INDENT=$(touch ran)\n");
        let go = resolve(&root.join("main.go"), None, Some(Language::Go));
        assert_eq!((go.style, go.size, go.tab_width), ((Style::Tab, Origin::Default), (4, Origin::Default), (4, Origin::Default)));
        let rs = resolve(&root.join("main.rs"), None, Some(Language::Rust));
        assert_eq!((rs.style, rs.size), ((Style::Space, Origin::Default), (4, Origin::Default)));
        let tabs = resolve(&root.join("main.rs"), Some(Indent::Tabs), Some(Language::Rust));
        assert_eq!((tabs.style, tabs.size), ((Style::Tab, Origin::Project), (4, Origin::Default)));
        let two = resolve(&root.join("main.go"), Some(Indent::Spaces(2)), Some(Language::Go));
        assert_eq!((two.style, two.size, two.tab_width), ((Style::Space, Origin::Project), (2, Origin::Project), (2, Origin::Project)));
        assert!(!root.join("ran").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "resolution created a file");
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        write(dir, name, "#!/bin/sh\ntouch \"$0.ran\"\n");
        let path = dir.join(name);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn formatters_resolve_from_project_files_and_path_without_running_anything() {
        let scratch = tempfile::tempdir().unwrap();
        let (root, bin) = (scratch.path().join("p"), scratch.path().join("bin"));
        write(&root, "rustfmt.toml", "max_width = 80\n");
        write(&root, "web/package.json", "{\"prettier\": {}}\n");
        write(&root, "pyproject.toml", "[tool.ruff]\nline-length = 90\n");
        write(&root, ".env", "PATH=$(touch ran)\n");
        let prettier = executable(&root, "web/node_modules/.bin/prettier");
        let rustfmt = executable(&bin, "rustfmt");
        write(&bin, "gofmt", "not executable");
        let path = std::env::join_paths([&bin]).unwrap();
        let path = Some(path.as_os_str());

        let rs = formatter(&root.join("src/a.rs"), &root, Some(Language::Rust), path).unwrap();
        assert_eq!(rs, Formatter { tool: "rustfmt", config: Some(root.join("rustfmt.toml")), executable: Some(rustfmt) });
        let ts = formatter(&root.join("web/src/a.tsx"), &root, Some(Language::Tsx), path).unwrap();
        assert_eq!(ts, Formatter { tool: "prettier", config: Some(root.join("web/package.json")), executable: Some(prettier.clone()) });
        let py = formatter(&root.join("a.py"), &root, Some(Language::Python), path).unwrap();
        assert_eq!(py, Formatter { tool: "ruff", config: Some(root.join("pyproject.toml")), executable: None });
        let go = formatter(&root.join("a.go"), &root, Some(Language::Go), path).unwrap();
        assert_eq!(go, Formatter { tool: "gofmt", config: None, executable: None }, "a non-executable file is not a tool");
        // Unconfigured project formatters are not guessed; outside the project's files nothing is consulted.
        assert_eq!(formatter(&root.join("a.rb"), &root, Some(Language::Ruby), path), None);
        assert_eq!(formatter(&scratch.path().join("x.js"), &root, Some(Language::JavaScript), path), None);
        assert_eq!(formatter(&root.join("a.txt"), &root, None, path), None);
        assert!(!root.join("ran").exists() && !prettier.with_extension("ran").exists() && !bin.join("rustfmt.ran").exists());
    }
}
