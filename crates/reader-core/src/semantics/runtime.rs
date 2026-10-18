//! Existing language-server runtimes. Probing and discovery run only on request, on session workers.
use std::path::{Path, PathBuf};
use super::{java, provider};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Java,
    PyrightNode,
}

impl Kind {
    pub fn label(self) -> &'static str { match self { Self::Java => "Java", Self::PyrightNode => "Node.js" } }
    pub fn input_label(self) -> &'static str { match self { Self::Java => "Java executable / JDK folder", Self::PyrightNode => "Node.js executable / install folder" } }
    pub fn provider(self) -> &'static str { match self { Self::Java => "jdtls", Self::PyrightNode => "pyright" } }
}

#[derive(Default)]
pub struct Runtime {
    pub kind: Kind,
    pub configured: Option<PathBuf>,
    pub candidates: Vec<PathBuf>,
    pub message: String,
}

impl Runtime {
    pub fn choices(&self, input: &str) -> Vec<(String, Option<PathBuf>)> {
        let entered = (!input.trim().is_empty()).then(|| PathBuf::from(input.trim()));
        let mut choices = vec![
            ("Save entered path and restart".into(), entered),
            (format!("Restore automatic {} selection and restart", self.kind.label()), None),
        ];
        choices.extend(self.candidates.iter().map(|p| (format!("Use {} and restart", p.display()), Some(p.clone()))));
        choices
    }
}

pub fn configured(kind: Kind, settings: &provider::Settings) -> Option<PathBuf> {
    let settings = settings.providers.get(kind.provider())?;
    match kind {
        Kind::Java => settings.args.as_deref().and_then(java::configured),
        Kind::PyrightNode => settings.node.clone(),
    }
}

pub fn discover(kind: Kind, current: Option<PathBuf>) -> Vec<PathBuf> {
    match kind {
        Kind::Java => java::discover(current),
        Kind::PyrightNode => {
            let mut homes = Vec::new();
            if let Some(home) = std::env::var_os("MISE_DATA_DIR") {
                homes.push(PathBuf::from(home).join("installs/node"));
            }
            if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
                homes.push(PathBuf::from(home).join("mise/installs/node"));
            }
            if let Some(home) = std::env::var_os("HOME") {
                let home = PathBuf::from(home);
                homes.extend([home.join(".local/share/mise/installs/node"), home.join(".nvm/versions/node")]);
            }
            if let Some(home) = std::env::var_os("NVM_DIR") {
                homes.push(PathBuf::from(home).join("versions/node"));
            }
            discover_node(current, std::env::var_os("PATH").as_deref(), &homes)
        },
    }
}

pub fn save(kind: Kind, file: &Path, path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    match kind {
        Kind::Java => java::save(file, path),
        Kind::PyrightNode => {
            let path = path.as_deref().map(node_path).transpose()?;
            if let Some(path) = &path {
                let version = version(path, "--version")?;
                let components: Vec<_> = version.strip_prefix('v').unwrap_or("").split('.').collect();
                if components.len() != 3 || !components.iter().all(|part| part.parse::<u32>().is_ok()) {
                    return Err(format!("{} did not report a Node.js version (expected vMAJOR.MINOR.PATCH): {version}", path.display()));
                }
            }
            java::update_settings(file, "pyright", "Node.js", |provider| {
                if let Some(path) = &path {
                    let custom = provider.get("executable").and_then(serde_json::Value::as_str)
                        .is_some_and(|name| name != "pyright-langserver");
                    if !custom && !path.parent().is_some_and(|bin| bin.join(server_name()).is_file()) {
                        return Err(format!("Pyright is not installed beside this Node.js executable ({}). Choose an existing Node.js installation containing Pyright, or install Pyright there yourself; Lenz never installs it.", path.display()));
                    }
                    provider.insert("node".into(), serde_json::json!(path));
                } else {
                    provider.remove("node");
                }
                serde_json::from_value::<provider::ProviderSettings>(serde_json::Value::Object(provider.clone()))
                    .map_err(|e| format!("Invalid Pyright provider settings: {e}"))?;
                Ok(())
            })?;
            Ok(path)
        },
    }
}

fn node_name() -> &'static str { if cfg!(windows) { "node.exe" } else { "node" } }
fn server_name() -> &'static str { if cfg!(windows) { "pyright-langserver.cmd" } else { "pyright-langserver" } }

fn node_path(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_dir() { path.join("bin").join(node_name()) } else { path.to_owned() };
    if !path.is_file() || path.file_name() != Some(std::ffi::OsStr::new(node_name())) {
        return Err(format!("Choose an existing Node.js executable ({}) or install folder: {}", node_name(), path.display()));
    }
    // Keep the bin folder of a symlinked node: this is where its matching server is installed.
    let parent = path.parent().unwrap_or(Path::new(".")).canonicalize()
        .map_err(|e| format!("Cannot open Node.js folder: {e}"))?;
    parent.to_str().ok_or("Node.js path must be valid UTF-8")?;
    std::env::join_paths([&parent]).map_err(|_| "Node.js folder cannot be represented in PATH")?;
    Ok(parent.join(node_name()))
}

fn discover_node(current: Option<PathBuf>, path: Option<&std::ffi::OsStr>, homes: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut identities = Vec::new();
    let mut add = |path: PathBuf| {
        if paths.len() < 64
            && let Ok(path) = node_path(&path)
            && path.parent().is_some_and(|bin| bin.join(server_name()).is_file())
            && let Ok(identity) = path.canonicalize()
            && !identities.contains(&identity)
        {
            identities.push(identity);
            paths.push(path);
        }
    };
    if let Some(current) = current { add(current); }
    if let Some(path) = path {
        for bin in std::env::split_paths(path).filter(|p| p.is_absolute()).take(256) { add(bin.join(node_name())); }
    }
    for home in homes {
        if let Ok(entries) = std::fs::read_dir(home) {
            let mut entries: Vec<_> = entries.take(256).filter_map(Result::ok).map(|e| e.path()).collect();
            entries.sort();
            for entry in entries { add(entry.join("bin").join(node_name())); }
        }
    }
    paths
}

/// Bounded stdout and stderr, without shell startup files or a long-lived version process.
pub fn version(path: &Path, flag: &str) -> Result<String, String> {
    version_in(path, flag, &[], None)
}

pub fn version_in(path: &Path, flag: &str, env: &[(String, String)], cwd: Option<&Path>) -> Result<String, String> {
    version_args_in(path, &[flag], env, cwd)
}

pub fn version_args_in(path: &Path, args: &[&str], env: &[(String, String)], cwd: Option<&Path>) -> Result<String, String> {
    let flags = args.join(" ");
    use std::{io::Read, process::{Command, Stdio}, time::{Duration, Instant}};
    let mut command = Command::new(path);
    command.args(args).envs(env.iter().map(|(key, value)| (key, value))).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd { command.current_dir(cwd); }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|e| format!("Cannot run {} {flags}: {e}", path.display()))?;
    fn capture(mut pipe: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (send, recv) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let mut truncated = false;
            let mut buffer = [0; 4096];
            while let Ok(count) = pipe.read(&mut buffer) {
                if count == 0 { break; }
                let kept = count.min(8192 - output.len());
                output.extend_from_slice(&buffer[..kept]);
                truncated |= kept < count;
            }
            if truncated { output.extend_from_slice(b"\n[version output truncated]\n"); }
            let _ = send.send(output);
        });
        recv
    }
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        super::kill_group(&mut child);
        let _ = child.wait();
        return Err("Cannot capture runtime version output".into());
    };
    let stdout = capture(stdout);
    let stderr = capture(stderr);
    let deadline = Instant::now() + Duration::from_secs(3);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => break Err(e.to_string()),
            _ if Instant::now() >= deadline => break Err(format!("{} {flags} did not finish within 3 seconds", path.display())),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    super::kill_group(&mut child);
    let _ = child.wait();
    let output: Vec<_> = [stdout, stderr].into_iter().flat_map(|pipe| pipe.recv_timeout(Duration::from_millis(100)).unwrap_or_default()).collect();
    let text = String::from_utf8_lossy(&output);
    let mut text = text.lines().map(|line| lenz_content::display::display_line(line).0).collect::<Vec<_>>().join("\n");
    let mut end = text.len().min(16384);
    while !text.is_char_boundary(end) { end -= 1; }
    text.truncate(end);
    let text = text.trim().to_owned();
    match result? {
        status if status.success() => Ok(text),
        status => Err(format!("{} {flags} failed: {status}\n{text}", path.display())),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt;

    fn node(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("Node with spaces/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let node = bin.join("node");
        publish(&node, &format!("#!/bin/sh\n{body}\n"));
        std::fs::write(bin.join("pyright-langserver"), "#!/usr/bin/env node\n").unwrap();
        node
    }

    fn publish(path: &Path, script: &str) {
        let temporary = path.with_extension("new");
        // Keep writable script descriptors out of children spawned by parallel tests.
        assert!(std::process::Command::new("/bin/sh")
            .args(["-c", "printf '%s' \"$1\" > \"$2\"", "fixture", script])
            .arg(&temporary).status().unwrap().success());
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(temporary, path).unwrap();
    }

    #[test]
    fn node_save_and_reset_preserve_original_launch_and_unknown_settings() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "test \"$1\" = --version && echo v26.8.1");
        let file = dir.path().join("providers.json");
        let mut expected = json!({"extra":true,"providers":{"pyright":{"args":["--stdio","--keep"],"env":{"PATH":"/old/bin","KEEP":"yes"},"settings":{"future":42}},"other":{"unknown":true}}});
        std::fs::write(&file, expected.to_string()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(save(Kind::PyrightNode, &file, Some(node.parent().unwrap().parent().unwrap().into())).unwrap(), Some(node.clone()));
        expected["providers"]["pyright"]["node"] = json!(node);
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap(), expected);
        assert_eq!(configured(Kind::PyrightNode, &provider::Settings::read(&file)), Some(node));
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o640);
        save(Kind::PyrightNode, &file, None).unwrap();
        expected["providers"]["pyright"].as_object_mut().unwrap().remove("node");
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap(), expected);
    }

    #[test]
    fn node_validation_and_malformed_settings_do_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "echo v26.8.1");
        let file = dir.path().join("providers.json");
        for text in ["{", "[]", r#"{"providers":[]}"#, r#"{"providers":{"pyright":[]}}"#, r#"{"providers":{"pyright":{"args":[1]}}}"#] {
            std::fs::write(&file, text).unwrap();
            assert!(save(Kind::PyrightNode, &file, Some(node.clone())).is_err());
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        }
        std::fs::write(&file, "{}").unwrap();
        assert!(save(Kind::PyrightNode, &file, Some(dir.path().join("missing"))).is_err());
        std::fs::remove_file(node.parent().unwrap().join("pyright-langserver")).unwrap();
        let error = save(Kind::PyrightNode, &file, Some(node)).unwrap_err();
        assert!(error.contains("Pyright") && error.contains("install"), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
    }
    #[test]
    fn runtime_versions_capture_stdout_and_stderr_with_bounded_output() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "printf 'v26.8.1\n'; printf 'runtime detail\n' >&2");
        let output = version(&node, "--version").unwrap();
        assert!(output.contains("v26.8.1") && output.contains("runtime detail"), "{output}");
        publish(&node, "#!/bin/sh\nyes x | head -c 100000\n");
        assert!(version(&node, "--version").unwrap().len() <= 16384);
    }

    #[test]
    fn node_timeout_and_failed_version_leave_settings_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "sleep 30");
        let file = dir.path().join("providers.json");
        std::fs::write(&file, "{}").unwrap();
        let start = std::time::Instant::now();
        let error = save(Kind::PyrightNode, &file, Some(node.clone())).unwrap_err();
        assert!(error.contains("3 seconds"), "{error}");
        assert!(start.elapsed() < std::time::Duration::from_secs(6));
        publish(&node, "#!/bin/sh\necho incompatible >&2; exit 13\n");
        let error = save(Kind::PyrightNode, &file, Some(node)).unwrap_err();
        assert!(error.contains("failed") && error.contains("incompatible"), "{error}");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "{}");
    }

    #[test]
    fn node_discovery_finds_existing_installs_and_deduplicates_paths() {
        let dir = tempfile::tempdir().unwrap();
        let first = node(&dir.path().join("mise/26.8.1"), "echo v26.8.1");
        let second = node(&dir.path().join("nvm/v24.0.0"), "echo v24.0.0");
        let missing = node(&dir.path().join("mise/26.10.0"), "echo v26.10.0");
        std::fs::remove_file(missing.parent().unwrap().join("pyright-langserver")).unwrap();
        // A manager's version directories contain bin/node directly.
        let homes = [dir.path().join("mise"), dir.path().join("nvm")];
        for executable in [&first, &second, &missing] {
            let home = executable.parent().unwrap().parent().unwrap();
            let install = home.parent().unwrap();
            std::os::unix::fs::symlink(home.join("bin"), install.join("bin")).unwrap();
        }
        let path = std::env::join_paths([first.parent().unwrap(), first.parent().unwrap(), missing.parent().unwrap()]).unwrap();
        let discovered = discover_node(Some(first.clone()), Some(&path), &homes);
        assert_eq!(discovered, [first, second]);
    }

    #[test]
    fn custom_pyright_executable_needs_only_existing_node_and_resets_without_losing_it() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "echo v26.8.1");
        std::fs::remove_file(node.parent().unwrap().join("pyright-langserver")).unwrap();
        let file = dir.path().join("providers.json");
        let original = json!({"providers":{"pyright":{"executable":"/custom/server","env":{"PATH":"/custom/bin"}}}});
        std::fs::write(&file, original.to_string()).unwrap();
        save(Kind::PyrightNode, &file, Some(node)).unwrap();
        save(Kind::PyrightNode, &file, None).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(file).unwrap()).unwrap(), original);
    }

    #[test]
    fn runtime_versions_use_the_provider_environment_and_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "printf '%s\\n' \"$LENZ_RUNTIME_FIXTURE\"; pwd");
        let output = version_in(&node, "--version", &[("LENZ_RUNTIME_FIXTURE".into(), "selected-provider".into())], Some(dir.path())).unwrap();
        assert_eq!(output.lines().next(), Some("selected-provider"));
        assert_eq!(PathBuf::from(output.lines().nth(1).unwrap()), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn node_rejects_a_different_executable_reporting_success() {
        let dir = tempfile::tempdir().unwrap();
        let node = node(dir.path(), "echo not-node");
        let file = dir.path().join("providers.json");
        std::fs::write(&file, "{}").unwrap();
        assert!(save(Kind::PyrightNode, &file, Some(node)).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "{}");
    }

    #[test]
    fn node_rejects_unrepresentable_paths_without_writing_settings() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("providers.json");
        std::fs::write(&file, "{}").unwrap();
        for folder in [std::ffi::OsString::from_vec(b"invalid-\xff".to_vec()), "path:separator".into()] {
            let node = node(&dir.path().join(folder), "echo v26.8.1");
            assert!(save(Kind::PyrightNode, &file, Some(node)).is_err());
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
        }
    }



}
