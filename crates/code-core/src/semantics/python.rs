//! The Python environment Pyright analyses a workspace with, as the project or Lenz's provider settings configure
//! it. Lenz reads the same settings Pyright reads, so it can say which environment answers and refuse to start
//! Pyright when the configured one is missing: Pyright itself would then fall back to the first interpreter on its
//! PATH without telling anyone (1.1.414 only logs it). Reads bounded project files; [`describe`] also runs an
//! interpreter's `--version` (a project-chosen one only in an opened folder the user trusts), so call it off the UI
//! thread.
use std::path::{Path, PathBuf};

use serde_json::Value;

/// An environment the configuration names, found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configured {
    /// The environment's folder (a virtual environment), when it is one.
    pub prefix: Option<PathBuf>,
    /// Its interpreter, when known.
    pub interpreter: Option<PathBuf>,
    /// Where its installed packages are (`site-packages`), when found.
    pub packages: Option<PathBuf>,
    /// Which setting names it, as the user is told it.
    pub source: String,
    /// Named by an absolute path in Lenz's provider settings: the user's own interpreter, not one the project chose
    /// (a relative `pythonPath` names a file of whichever project is open).
    pub own: bool,
}

impl Configured {
    /// The folder holding its command-line tools (`bin`, `Scripts` on Windows): where its formatters are.
    pub fn tools(&self) -> Option<PathBuf> {
        self.prefix.as_ref().map(|p| p.join(BIN)).or_else(|| self.interpreter.as_deref().and_then(Path::parent).map(Path::to_owned))
    }
}

const BIN: &str = if cfg!(windows) { "Scripts" } else { "bin" };
const AFTER: &str = "then choose Restart Language Server";

/// The environment configured for Pyright's workspace `root`: from `pyrightconfig.json`, else `[tool.pyright]` in
/// `pyproject.toml` (`venvPath` and `venv`), else `pythonPath` in the provider's `settings` (its `python` section).
/// `Ok(None)`: none is configured. `Err`: one is configured but missing or unreadable, with what to do. No
/// configuration above `within` (the opened folder holding the root, else the root) is read.
pub fn configured(root: &Path, settings: &Value, within: &Path) -> Result<Option<Configured>, String> {
    let setting = |key: &str| settings.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
    if let Some((file, venv_path, venv)) = config(root, within)? && let Some(venv) = venv {
        let base = file.parent().unwrap_or(root);
        let (folder, named) = match (&venv_path, setting("venvPath")) {
            (Some(p), _) => (base.join(p), p.clone()),
            (None, Some(p)) => (root.join(p), p.to_owned()),
            (None, None) => return Err(format!("venv \"{venv}\" is set without venvPath in {}, so Pyright ignores it and would use the first python3 on its PATH instead. Set venvPath (usually \".\") beside it, {AFTER}", file.display())),
        };
        let prefix = super::provider::normalize(&folder.join(&venv));
        let source = format!("venv \"{venv}\" with venvPath \"{named}\" in {}", file.display());
        if !prefix.is_dir() {
            return Err(format!("The configured Python environment {} does not exist ({source}). Lenz does not start Pyright with another environment: create it (for example `python3 -m venv {}`) or correct the setting, {AFTER}", prefix.display(), prefix.display()));
        }
        let Some(packages) = site_packages(&prefix) else {
            return Err(format!("{} is not a Python environment: it has no lib/python3.*/site-packages folder ({source}). Lenz does not start Pyright with another environment: recreate it or correct the setting, {AFTER}", prefix.display()));
        };
        let interpreter = prefix.join(BIN).join(if cfg!(windows) { "python.exe" } else { "python" });
        return Ok(Some(Configured { prefix: Some(prefix), interpreter: Some(interpreter), packages: Some(packages), source, own: false }));
    }
    let Some(python) = setting("pythonPath") else { return Ok(None) };
    let own = Path::new(python).is_absolute();
    let python = root.join(python);
    let source = "python.pythonPath in Lenz's provider settings".to_owned();
    if !python.is_file() {
        return Err(format!("The configured Python interpreter {} does not exist ({source}). Lenz does not start Pyright with another interpreter: correct the setting, {AFTER}", python.display()));
    }
    // An interpreter inside a virtual environment (`<prefix>/bin/python`).
    let prefix = python.parent().and_then(Path::parent).filter(|p| p.join("pyvenv.cfg").is_file()).map(Path::to_owned);
    let packages = prefix.as_deref().and_then(site_packages);
    Ok(Some(Configured { prefix, interpreter: Some(python), packages, source, own }))
}

/// One line for Server Details: the environment Pyright uses for `root` (a configured one, else the first `python3`,
/// else `python`, on `path`), its Python version, and an unconfigured project environment it does not use. An
/// interpreter the project chose runs for its version only inside `trusted`, the opened folder the user trusts. `Err`
/// as [`configured`]'s.
pub fn describe(root: &Path, settings: &Value, path: Option<&std::ffi::OsStr>, trusted: Option<&Path>) -> Result<String, String> {
    // The project's interpreter runs only inside the trusted folder, its links resolved (a `..` or a link out of it
    // is not inside).
    let inside = |python: &Path, t: &Path| std::fs::canonicalize(python).ok().zip(std::fs::canonicalize(t).ok()).is_some_and(|(p, t)| p.starts_with(t));
    // Asked outside the project: a version manager's shim would read the project's `.python-version`.
    let version = |python: &Path| super::runtime::version_in(python, "--version", &[], Some(Path::new(super::runtime::NEUTRAL))).map(|v| v.lines().next().unwrap_or_default().trim().to_owned()).unwrap_or_else(|e| format!("Python version unknown ({})", e.lines().next().unwrap_or_default()));
    // In a folder that is not trusted, Pyright is not given a relative pythonPath (see [`untrusted`]): neither is this.
    let withheld = trusted.is_none() && untrusted(settings.clone()) != *settings;
    let settings = &if trusted.is_none() { untrusted(settings.clone()) } else { settings.clone() };
    let note = if withheld { " python.pythonPath names a file of this project, which Pyright is not given in a folder you do not trust: choose Restricted in the status bar, or Trust Project Tools in This Folder, to use it." } else { "" };
    if let Some(c) = configured(root, settings, trusted.unwrap_or(root))? {
        let recorded = c.prefix.as_deref().and_then(recorded_version);
        let version = recorded.unwrap_or_else(|| match c.interpreter.as_deref() {
            None => "Python version unknown".into(),
            // The project's choice of interpreter is the project's program.
            Some(python) if c.own || trusted.is_some_and(|t| inside(python, t)) => version(python),
            Some(_) if trusted.is_some() => "Python version unknown (its pyvenv.cfg records none, and Lenz does not run a project's interpreter outside the trusted folder)".into(),
            Some(_) => "Python version unknown (its pyvenv.cfg records none, and Lenz runs a project's interpreter only in a folder you trust: choose Restricted in the status bar, or Trust Project Tools in This Folder)".into(),
        });
        let mut line = match &c.prefix {
            Some(prefix) => format!("Python environment: {} ({}), {version}", prefix.display(), c.source),
            None => format!("Python environment: interpreter {} ({}), {version}", c.interpreter.as_deref().unwrap_or(Path::new("")).display(), c.source),
        };
        if let Some(packages) = &c.packages { line.push_str(&format!("; installed packages from {}", packages.display())); }
        return Ok(line);
    }
    let found = ["python3", "python"].iter().find_map(|name| super::provider::find_executable(Path::new(name), path));
    let mut line = match found {
        Some(python) => format!("Python environment: none configured, so Pyright uses the first python3 (else python) on its PATH: {} ({}).", python.display(), version(&python)),
        None => "Python environment: none configured, and no python3 or python is on Pyright's PATH, so it resolves only the standard library's bundled stubs and the project's own files.".into(),
    };
    if let Some(unused) = [".venv", "venv"].iter().map(|n| root.join(n)).find(|d| d.join("pyvenv.cfg").is_file()) {
        line.push_str(&format!(" {} holds a virtual environment that is not configured, so Pyright does not use it.", unused.display()));
    }
    line.push_str(" To use a project environment, set venvPath and venv in pyrightconfig.json or [tool.pyright] in pyproject.toml, or python.pythonPath in Lenz's provider settings");
    line.push_str(note);
    Ok(line)
}

/// Pyright's settings as sent in a folder that is not trusted: a relative `pythonPath` (a file of the project) is left
/// out, so that Pyright never runs an interpreter the project supplies there (it uses the first python3 on its PATH).
pub fn untrusted(mut settings: Value) -> Value {
    if let Some(o) = settings.as_object_mut() && o.get("pythonPath").and_then(Value::as_str).is_some_and(|p| Path::new(p).is_relative()) {
        o.remove("pythonPath");
    }
    settings
}

/// Pyright's settings as sent for workspace `root`: a relative `pythonPath` (a file of the project) is left out unless
/// it names a file inside `trusted`, where the project's own programs may run (`None`: nowhere; see [`untrusted`]), so
/// that Pyright never runs an interpreter a folder the user does not trust supplies.
pub fn sent(mut settings: Value, root: &Path, trusted: Option<&Path>) -> Value {
    let outside = |p: &str| Path::new(p).is_relative() && !trusted.is_some_and(|t| super::provider::normalize(&root.join(p)).starts_with(t));
    if let Some(o) = settings.as_object_mut() && o.get("pythonPath").and_then(Value::as_str).is_some_and(outside) {
        o.remove("pythonPath");
    }
    settings
}

/// The configuration file Pyright reads for `root` and its `venvPath` and `venv`: `pyrightconfig.json` there, else
/// `pyproject.toml` there with `[tool.pyright]`; a `pyproject.toml` without that section sends Pyright 1.1.414 to
/// the nearest `pyrightconfig.json` above (seen in its log), so it does here.
type Config = (PathBuf, Option<String>, Option<String>);
fn config(root: &Path, within: &Path) -> Result<Option<Config>, String> {
    let unreadable = |file: &Path, e: String| format!("Cannot read {} ({e}), so Lenz cannot tell which environment Pyright would use: correct it, {AFTER}", file.display());
    let json = |file: PathBuf| -> Result<Option<Config>, String> {
        let text = read(&file).map_err(|e| unreadable(&file, e))?;
        let value: Value = serde_json::from_str(&strip_jsonc(&text)).map_err(|e| unreadable(&file, e.to_string()))?;
        let get = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
        Ok(Some((file.clone(), get("venvPath"), get("venv"))))
    };
    let own = root.join("pyrightconfig.json");
    if own.is_file() { return json(own); }
    let pyproject = root.join("pyproject.toml");
    if !pyproject.is_file() { return Ok(None); }
    let text = read(&pyproject).map_err(|e| unreadable(&pyproject, e))?;
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| unreadable(&pyproject, e.message().to_owned()))?;
    if let Some(section) = table.get("tool").and_then(|t| t.get("pyright")) {
        let get = |k: &str| section.get(k).and_then(toml::Value::as_str).map(str::to_owned);
        return Ok(Some((pyproject, get("venvPath"), get("venv"))));
    }
    match root.ancestors().skip(1).take_while(|d| d.starts_with(within)).map(|d| d.join("pyrightconfig.json")).find(|f| f.is_file()) {
        Some(above) => json(above),
        None => Ok(None),
    }
}

/// A project file of at most 1 MiB.
fn read(file: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(file).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > 1 << 20 { return Err("not a regular file of at most 1 MiB".into()); }
    std::fs::read_to_string(file).map_err(|e| e.to_string())
}

/// JSON with comments and trailing commas (as Pyright reads its configuration) as plain JSON.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut string = false;
    while let Some(c) = chars.next() {
        if string {
            out.push(c);
            match c {
                '\\' => { if let Some(n) = chars.next() { out.push(n); } }
                '"' => string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => { string = true; out.push(c); }
            ('/', Some('/')) => { while chars.next_if(|n| *n != '\n').is_some() {} }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() { if last == '*' && n == '/' { break; } last = n; }
                out.push(' ');
            }
            (',', _) => {
                // A comma whose next significant character closes the object or array is dropped.
                if !matches!(significant(chars.clone()), Some('}' | ']')) { out.push(c); }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The next character that is not white space or inside a comment.
fn significant(mut chars: std::iter::Peekable<std::str::Chars<'_>>) -> Option<char> {
    loop {
        let c = chars.next()?;
        match (c, chars.peek()) {
            (c, _) if c.is_whitespace() => {}
            ('/', Some('/')) => { while chars.next_if(|n| *n != '\n').is_some() {} }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() { if last == '*' && n == '/' { break; } last = n; }
            }
            (c, _) => return Some(c),
        }
    }
}

/// `prefix`'s installed-packages folder.
fn site_packages(prefix: &Path) -> Option<PathBuf> {
    let windows = prefix.join("Lib/site-packages");
    if windows.is_dir() { return Some(windows); }
    let mut versions: Vec<PathBuf> = std::fs::read_dir(prefix.join("lib")).ok()?.take(64).filter_map(Result::ok)
        .map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("python3"))).collect();
    versions.sort();
    versions.into_iter().map(|v| v.join("site-packages")).find(|p| p.is_dir())
}

/// The Python version a virtual environment records (`pyvenv.cfg`: `version`, or uv's `version_info`).
fn recorded_version(prefix: &Path) -> Option<String> {
    let text = read(&prefix.join("pyvenv.cfg")).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        matches!(k.trim(), "version" | "version_info").then(|| format!("Python {}", v.trim()))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(root: &Path, name: &str, text: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// An executable script, written by another process so that no test's children inherit a writable descriptor.
    fn script(root: &Path, name: &str, body: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(std::process::Command::new("/bin/sh").args(["-c", "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"", "script", body]).arg(&path).status().unwrap().success());
        path
    }

    /// A virtual environment's layout (what `python3 -m venv` writes, without its interpreter).
    fn venv(root: &Path, name: &str, version: &str) -> PathBuf {
        write(root, &format!("{name}/pyvenv.cfg"), &format!("home = /usr/bin\ninclude-system-site-packages = false\nversion = {version}\n"));
        std::fs::create_dir_all(root.join(name).join("lib/python3.13/site-packages")).unwrap();
        root.join(name)
    }

    #[test]
    fn the_project_configuration_names_the_environment_and_its_packages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let env = venv(root, "env", "3.13.1");
        // JSON with comments and a trailing comma, as Pyright accepts it.
        let config = write(root, "pyrightconfig.json", "{\n  // the project's own\n  \"venvPath\": \".\", /* here */ \"venv\": \"env\",\n}\n");
        let found = configured(root, &Value::Null, root).unwrap().unwrap();
        assert_eq!(found.prefix.as_deref(), Some(env.as_path()));
        assert_eq!(found.packages, Some(env.join("lib/python3.13/site-packages")));
        assert_eq!(found.interpreter, Some(env.join("bin/python")));
        assert_eq!(found.source, format!("venv \"env\" with venvPath \".\" in {}", config.display()));
        assert_eq!(found.tools(), Some(env.join("bin")));
        // The project's configuration wins over Lenz's pythonPath, as in Pyright.
        let other = write(root, "other/bin/python", "");
        let settings = json!({ "pythonPath": other });
        assert_eq!(configured(root, &settings, root).unwrap().unwrap().prefix.as_deref(), Some(env.as_path()));
        let line = describe(root, &settings, None, None).unwrap();
        assert_eq!(line, format!("Python environment: {} (venv \"env\" with venvPath \".\" in {}), Python 3.13.1; installed packages from {}", env.display(), config.display(), env.join("lib/python3.13/site-packages").display()));
    }

    #[test]
    fn pyproject_tool_pyright_configures_it_unless_a_pyrightconfig_does() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let dev = venv(root, "envs/dev", "3.12.4");
        let pyproject = write(root, "pyproject.toml", "[project]\nname = \"x\"\n\n[tool.pyright]\nvenvPath = \"envs\"\nvenv = \"dev\"\n");
        let found = configured(root, &Value::Null, root).unwrap().unwrap();
        assert_eq!((found.prefix.as_deref(), found.source.as_str()), (Some(dev.as_path()), format!("venv \"dev\" with venvPath \"envs\" in {}", pyproject.display()).as_str()));
        // `venv` alone takes its folder from Lenz's `venvPath` setting (relative to the workspace).
        write(root, "pyproject.toml", "[tool.pyright]\nvenv = \"dev\"\n");
        let found = configured(root, &json!({ "venvPath": "envs" }), root).unwrap().unwrap();
        assert_eq!(found.prefix.as_deref(), Some(dev.as_path()));
        let none = configured(root, &Value::Null, root).unwrap_err();
        assert!(none.contains("venv \"dev\" is set without venvPath") && none.contains("Pyright ignores it"), "{none}");
        // A pyproject without the section configures nothing; a pyrightconfig.json beside it wins.
        write(root, "pyproject.toml", "[project]\nname = \"x\"\n");
        assert_eq!(configured(root, &Value::Null, root).unwrap(), None);
        venv(root, "local", "3.13.0");
        write(root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"local\" }");
        assert_eq!(configured(root, &Value::Null, root).unwrap().unwrap().prefix, Some(root.join("local")));
    }

    #[test]
    fn a_missing_or_unreadable_environment_is_refused_with_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let config = write(root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"gone\" }");
        let why = configured(root, &Value::Null, root).unwrap_err();
        assert_eq!(why, format!("The configured Python environment {} does not exist (venv \"gone\" with venvPath \".\" in {}). Lenz does not start Pyright with another environment: create it (for example `python3 -m venv {}`) or correct the setting, then choose Restart Language Server", root.join("gone").display(), config.display(), root.join("gone").display()));
        assert_eq!(describe(root, &Value::Null, None, None).unwrap_err(), why);
        std::fs::create_dir(root.join("gone")).unwrap();
        let why = configured(root, &Value::Null, root).unwrap_err();
        assert!(why.starts_with(&format!("{} is not a Python environment: it has no lib/python3.*/site-packages folder", root.join("gone").display())), "{why}");
        write(root, "pyrightconfig.json", "{ \"venv\": ");
        let why = configured(root, &Value::Null, root).unwrap_err();
        assert!(why.starts_with(&format!("Cannot read {}", config.display())) && why.contains("which environment Pyright would use"), "{why}");
        std::fs::remove_file(&config).unwrap();
        let missing = root.join("nowhere/bin/python");
        let why = configured(root, &json!({ "pythonPath": missing }), root).unwrap_err();
        assert!(why.starts_with(&format!("The configured Python interpreter {} does not exist (python.pythonPath in Lenz's provider settings)", missing.display())), "{why}");
    }

    #[test]
    fn a_python_path_names_an_interpreter_and_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let env = venv(root, "venv", "3.13.2");
        let python = write(root, "venv/bin/python", "");
        let found = configured(root, &json!({ "pythonPath": python }), root).unwrap().unwrap();
        assert_eq!(found, Configured { prefix: Some(env.clone()), interpreter: Some(python.clone()), packages: Some(env.join("lib/python3.13/site-packages")), source: "python.pythonPath in Lenz's provider settings".into(), own: true });
        assert_eq!(found.tools(), Some(env.join("bin")));
        // An interpreter outside a virtual environment: its version is asked.
        let system = script(root, "system/bin/python3", "#!/bin/sh\necho 'Python 3.11.9'\n");
        let found = configured(root, &json!({ "pythonPath": system }), root).unwrap().unwrap();
        assert_eq!((found.prefix, found.packages), (None, None));
        assert_eq!(describe(root, &json!({ "pythonPath": system }), None, None).unwrap(), format!("Python environment: interpreter {} (python.pythonPath in Lenz's provider settings), Python 3.11.9", system.display()));
    }

    #[test]
    fn a_project_chosen_interpreter_runs_only_in_a_trusted_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let marker = dir.path().join("ran");
        // A configured environment without a recorded version, whose interpreter is the project's script.
        write(&root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"env\" }");
        std::fs::create_dir_all(root.join("env/lib/python3.12/site-packages")).unwrap();
        script(&root, "env/bin/python", &format!("#!/bin/sh\ntouch '{}'\necho 'Python 3.12.0'\n", marker.display()));
        let line = describe(&root, &Value::Null, None, None).unwrap();
        assert!(line.contains("Python version unknown") && line.contains("Trust Project Tools in This Folder"), "{line}");
        assert!(!marker.exists(), "an untrusted project's interpreter ran");
        // A folder that is not the trusted one (or holds it) does not trust it.
        let line = describe(&root, &Value::Null, None, Some(&root.join("elsewhere"))).unwrap();
        assert!(!marker.exists(), "a project outside the trusted folder ran");
        assert!(line.contains("outside the trusted folder"), "{line}");
        // A relative pythonPath names the open project's file: the project's program too.
        std::fs::remove_file(root.join("pyrightconfig.json")).unwrap();
        let relative = describe(&root, &json!({ "pythonPath": "env/bin/python" }), None, None).unwrap();
        assert!(relative.contains("Trust Project Tools in This Folder") && !marker.exists(), "{relative}");
        assert!(relative.contains("python.pythonPath names a file of this project, which Pyright is not given"), "{relative}");
        // Even when that file is missing: it is not given, so it does not stop Pyright.
        assert!(describe(&root, &json!({ "pythonPath": "gone/bin/python" }), None, None).is_ok());
        write(&root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"env\" }");
        let line = describe(&root, &Value::Null, None, Some(&root)).unwrap();
        assert!(line.contains("Python 3.12.0") && marker.exists(), "{line}");
    }

    #[test]
    fn without_configuration_pyright_uses_the_path_interpreter_and_an_unconfigured_venv_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        venv(&root, ".venv", "3.13.3");
        write(&root, "pyproject.toml", "[project]\nname = \"x\"\n");
        let bin = dir.path().join("bin");
        let python3 = script(&bin, "python3", "#!/bin/sh\necho 'Python 3.10.1'\n");
        let path = std::env::join_paths([&bin]).unwrap();
        assert_eq!(configured(&root, &Value::Null, &root).unwrap(), None);
        assert_eq!(describe(&root, &Value::Null, Some(&path), None).unwrap(), format!("Python environment: none configured, so Pyright uses the first python3 (else python) on its PATH: {} (Python 3.10.1). {} holds a virtual environment that is not configured, so Pyright does not use it. To use a project environment, set venvPath and venv in pyrightconfig.json or [tool.pyright] in pyproject.toml, or python.pythonPath in Lenz's provider settings", python3.display(), root.join(".venv").display()));
        let empty = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([empty.path()]).unwrap();
        let line = describe(empty.path(), &Value::Null, Some(&path), None).unwrap();
        assert!(line.starts_with("Python environment: none configured, and no python3 or python is on Pyright's PATH, so it resolves only the standard library's bundled stubs and the project's own files."), "{line}");
    }

    #[test]
    fn an_environment_outside_the_trusted_folder_is_not_run_whatever_its_path_says() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let marker = dir.path().join("ran");
        std::fs::create_dir_all(dir.path().join("outside/lib/python3.12/site-packages")).unwrap();
        script(dir.path(), "outside/bin/python", &format!("#!/bin/sh\ntouch '{}'\necho 'Python 3.12.0'\n", marker.display()));
        // A venvPath of `..`: the environment beside the trusted folder.
        write(&root, "pyrightconfig.json", "{ \"venvPath\": \"..\", \"venv\": \"outside\" }");
        let line = describe(&root, &Value::Null, None, Some(&root)).unwrap();
        assert!(line.contains("outside the trusted folder") && !marker.exists(), "{line}");
        // A link inside it to an environment outside.
        std::os::unix::fs::symlink(dir.path().join("outside"), root.join("linked")).unwrap();
        write(&root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"linked\" }");
        let line = describe(&root, &Value::Null, None, Some(&root)).unwrap();
        assert!(line.contains("outside the trusted folder") && !marker.exists(), "{line}");
    }

    #[test]
    fn a_pyrightconfig_above_the_root_is_read_only_within_the_opened_folder() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("outer");
        let root = outer.join("inner");
        venv(&outer, "env", "3.13.2");
        write(&outer, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"env\" }");
        write(&root, "pyproject.toml", "[project]\nname = \"x\"\n");
        assert_eq!(configured(&root, &Value::Null, &root).unwrap(), None, "above the opened folder");
        assert_eq!(configured(&root, &Value::Null, &outer).unwrap().and_then(|c| c.prefix), Some(outer.join("env")), "within it");
    }

    #[test]
    fn a_relative_interpreter_is_not_given_to_pyright_in_a_folder_that_is_not_trusted() {
        assert_eq!(untrusted(json!({ "pythonPath": "env/bin/python", "analysis": { "typeCheckingMode": "basic" } })), json!({ "analysis": { "typeCheckingMode": "basic" } }));
        assert_eq!(untrusted(json!({ "pythonPath": "/usr/bin/python3" })), json!({ "pythonPath": "/usr/bin/python3" }), "the user's own interpreter");
        assert_eq!(untrusted(Value::Null), Value::Null);
    }

    #[test]
    fn the_path_interpreters_version_is_asked_outside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        // A version manager's shim reads the folder it runs in (`.python-version`): it runs in none of the project's.
        let bin = dir.path().join("bin");
        script(&bin, "python3", "#!/bin/sh\necho \"Python 3.10.1 in $(pwd)\"\n");
        let path = std::env::join_paths([&bin]).unwrap();
        let line = describe(&root, &Value::Null, Some(&path), None).unwrap();
        assert!(line.contains(&format!("(Python 3.10.1 in {})", super::super::runtime::NEUTRAL)), "{line}");
    }

    /// A relative pythonPath is given to Pyright only inside the folder where the project's own programs may run.
    #[test]
    fn a_relative_python_path_is_sent_only_inside_the_trusted_bound() {
        let settings = serde_json::json!({ "pythonPath": "../e/bin/python" });
        let root = Path::new("/m/svc");
        assert_eq!(sent(settings.clone(), root, None).get("pythonPath"), None);
        assert_eq!(sent(settings.clone(), root, Some(Path::new("/m/svc"))).get("pythonPath"), None, "outside the trusted folder");
        assert!(sent(settings.clone(), root, Some(Path::new("/m"))).get("pythonPath").is_some());
        let inside = serde_json::json!({ "pythonPath": ".venv/bin/python" });
        assert!(sent(inside.clone(), root, Some(root)).get("pythonPath").is_some());
        let own = serde_json::json!({ "pythonPath": "/usr/bin/python3" });
        assert!(sent(own, root, None).get("pythonPath").is_some(), "the user's own interpreter");
    }
}
