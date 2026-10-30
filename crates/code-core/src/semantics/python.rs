//! The Python environment Pyright analyses a workspace with, as the project or Lenz's provider settings configure
//! it. Lenz reads the same settings Pyright reads, so it can say which environment answers and refuse to start
//! Pyright when the configured one is missing: Pyright itself would then fall back to the first interpreter on its
//! PATH without telling anyone (1.1.414 only logs it). Reads bounded project files; [`describe`] also runs an
//! interpreter's `--version`, so call it off the UI thread.
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
/// `Ok(None)`: none is configured. `Err`: one is configured but missing or unreadable, with what to do.
pub fn configured(root: &Path, settings: &Value) -> Result<Option<Configured>, String> {
    let setting = |key: &str| settings.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
    if let Some((file, venv_path, venv)) = config(root)? && let Some(venv) = venv {
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
        return Ok(Some(Configured { prefix: Some(prefix), interpreter: Some(interpreter), packages: Some(packages), source }));
    }
    let Some(python) = setting("pythonPath") else { return Ok(None) };
    let python = root.join(python);
    let source = "python.pythonPath in Lenz's provider settings".to_owned();
    if !python.is_file() {
        return Err(format!("The configured Python interpreter {} does not exist ({source}). Lenz does not start Pyright with another interpreter: correct the setting, {AFTER}", python.display()));
    }
    // An interpreter inside a virtual environment (`<prefix>/bin/python`).
    let prefix = python.parent().and_then(Path::parent).filter(|p| p.join("pyvenv.cfg").is_file()).map(Path::to_owned);
    let packages = prefix.as_deref().and_then(site_packages);
    Ok(Some(Configured { prefix, interpreter: Some(python), packages, source }))
}

/// One line for Server Details: the environment Pyright uses for `root` (a configured one, else the first `python3`,
/// else `python`, on `path`), its Python version, and an unconfigured project environment it does not use. `Err`
/// as [`configured`]'s.
pub fn describe(root: &Path, settings: &Value, path: Option<&std::ffi::OsStr>) -> Result<String, String> {
    let version = |python: &Path| super::runtime::version_in(python, "--version", &[], Some(root)).map(|v| v.lines().next().unwrap_or_default().trim().to_owned()).unwrap_or_else(|e| format!("Python version unknown ({})", e.lines().next().unwrap_or_default()));
    if let Some(c) = configured(root, settings)? {
        let recorded = c.prefix.as_deref().and_then(recorded_version);
        let version = recorded.unwrap_or_else(|| c.interpreter.as_deref().map_or_else(|| "Python version unknown".into(), version));
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
    Ok(line)
}

/// The configuration file Pyright reads for `root` and its `venvPath` and `venv`: `pyrightconfig.json` there, else
/// `pyproject.toml` there with `[tool.pyright]`; a `pyproject.toml` without that section sends Pyright 1.1.414 to
/// the nearest `pyrightconfig.json` above (seen in its log), so it does here.
type Config = (PathBuf, Option<String>, Option<String>);
fn config(root: &Path) -> Result<Option<Config>, String> {
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
    match root.ancestors().skip(1).map(|d| d.join("pyrightconfig.json")).find(|f| f.is_file()) {
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
        let found = configured(root, &Value::Null).unwrap().unwrap();
        assert_eq!(found.prefix.as_deref(), Some(env.as_path()));
        assert_eq!(found.packages, Some(env.join("lib/python3.13/site-packages")));
        assert_eq!(found.interpreter, Some(env.join("bin/python")));
        assert_eq!(found.source, format!("venv \"env\" with venvPath \".\" in {}", config.display()));
        assert_eq!(found.tools(), Some(env.join("bin")));
        // The project's configuration wins over Lenz's pythonPath, as in Pyright.
        let other = write(root, "other/bin/python", "");
        let settings = json!({ "pythonPath": other });
        assert_eq!(configured(root, &settings).unwrap().unwrap().prefix.as_deref(), Some(env.as_path()));
        let line = describe(root, &settings, None).unwrap();
        assert_eq!(line, format!("Python environment: {} (venv \"env\" with venvPath \".\" in {}), Python 3.13.1; installed packages from {}", env.display(), config.display(), env.join("lib/python3.13/site-packages").display()));
    }

    #[test]
    fn pyproject_tool_pyright_configures_it_unless_a_pyrightconfig_does() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let dev = venv(root, "envs/dev", "3.12.4");
        let pyproject = write(root, "pyproject.toml", "[project]\nname = \"x\"\n\n[tool.pyright]\nvenvPath = \"envs\"\nvenv = \"dev\"\n");
        let found = configured(root, &Value::Null).unwrap().unwrap();
        assert_eq!((found.prefix.as_deref(), found.source.as_str()), (Some(dev.as_path()), format!("venv \"dev\" with venvPath \"envs\" in {}", pyproject.display()).as_str()));
        // `venv` alone takes its folder from Lenz's `venvPath` setting (relative to the workspace).
        write(root, "pyproject.toml", "[tool.pyright]\nvenv = \"dev\"\n");
        let found = configured(root, &json!({ "venvPath": "envs" })).unwrap().unwrap();
        assert_eq!(found.prefix.as_deref(), Some(dev.as_path()));
        let none = configured(root, &Value::Null).unwrap_err();
        assert!(none.contains("venv \"dev\" is set without venvPath") && none.contains("Pyright ignores it"), "{none}");
        // A pyproject without the section configures nothing; a pyrightconfig.json beside it wins.
        write(root, "pyproject.toml", "[project]\nname = \"x\"\n");
        assert_eq!(configured(root, &Value::Null).unwrap(), None);
        venv(root, "local", "3.13.0");
        write(root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"local\" }");
        assert_eq!(configured(root, &Value::Null).unwrap().unwrap().prefix, Some(root.join("local")));
    }

    #[test]
    fn a_missing_or_unreadable_environment_is_refused_with_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let config = write(root, "pyrightconfig.json", "{ \"venvPath\": \".\", \"venv\": \"gone\" }");
        let why = configured(root, &Value::Null).unwrap_err();
        assert_eq!(why, format!("The configured Python environment {} does not exist (venv \"gone\" with venvPath \".\" in {}). Lenz does not start Pyright with another environment: create it (for example `python3 -m venv {}`) or correct the setting, then choose Restart Language Server", root.join("gone").display(), config.display(), root.join("gone").display()));
        assert_eq!(describe(root, &Value::Null, None).unwrap_err(), why);
        std::fs::create_dir(root.join("gone")).unwrap();
        let why = configured(root, &Value::Null).unwrap_err();
        assert!(why.starts_with(&format!("{} is not a Python environment: it has no lib/python3.*/site-packages folder", root.join("gone").display())), "{why}");
        write(root, "pyrightconfig.json", "{ \"venv\": ");
        let why = configured(root, &Value::Null).unwrap_err();
        assert!(why.starts_with(&format!("Cannot read {}", config.display())) && why.contains("which environment Pyright would use"), "{why}");
        std::fs::remove_file(&config).unwrap();
        let missing = root.join("nowhere/bin/python");
        let why = configured(root, &json!({ "pythonPath": missing })).unwrap_err();
        assert!(why.starts_with(&format!("The configured Python interpreter {} does not exist (python.pythonPath in Lenz's provider settings)", missing.display())), "{why}");
    }

    #[test]
    fn a_python_path_names_an_interpreter_and_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let env = venv(root, "venv", "3.13.2");
        let python = write(root, "venv/bin/python", "");
        let found = configured(root, &json!({ "pythonPath": python })).unwrap().unwrap();
        assert_eq!(found, Configured { prefix: Some(env.clone()), interpreter: Some(python.clone()), packages: Some(env.join("lib/python3.13/site-packages")), source: "python.pythonPath in Lenz's provider settings".into() });
        assert_eq!(found.tools(), Some(env.join("bin")));
        // An interpreter outside a virtual environment: its version is asked.
        let system = script(root, "system/bin/python3", "#!/bin/sh\necho 'Python 3.11.9'\n");
        let found = configured(root, &json!({ "pythonPath": system })).unwrap().unwrap();
        assert_eq!((found.prefix, found.packages), (None, None));
        assert_eq!(describe(root, &json!({ "pythonPath": system }), None).unwrap(), format!("Python environment: interpreter {} (python.pythonPath in Lenz's provider settings), Python 3.11.9", system.display()));
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
        assert_eq!(configured(&root, &Value::Null).unwrap(), None);
        assert_eq!(describe(&root, &Value::Null, Some(&path)).unwrap(), format!("Python environment: none configured, so Pyright uses the first python3 (else python) on its PATH: {} (Python 3.10.1). {} holds a virtual environment that is not configured, so Pyright does not use it. To use a project environment, set venvPath and venv in pyrightconfig.json or [tool.pyright] in pyproject.toml, or python.pythonPath in Lenz's provider settings", python3.display(), root.join(".venv").display()));
        let empty = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([empty.path()]).unwrap();
        let line = describe(empty.path(), &Value::Null, Some(&path)).unwrap();
        assert!(line.starts_with("Python environment: none configured, and no python3 or python is on Pyright's PATH, so it resolves only the standard library's bundled stubs and the project's own files."), "{line}");
    }
}
