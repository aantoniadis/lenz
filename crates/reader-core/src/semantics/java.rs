//! JDT LS runtime selection. Discovery and validation run only when requested, on the session's workers.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub fn configured(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .enumerate()
        .filter_map(|(i, arg)| {
            arg.strip_prefix("--java-executable=").or_else(|| {
                (arg == "--java-executable")
                    .then(|| args.get(i + 1).map(String::as_str))
                    .flatten()
            })
        })
        .next_back()
        .map(PathBuf::from)
}

pub fn discover(current: Option<PathBuf>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut add = |path: PathBuf| {
        if let Ok(path) = path.canonicalize()
            && path.is_file()
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    };
    if let Some(current) = current {
        add(current);
    }
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        add(PathBuf::from(home).join("bin").join(executable));
    }
    if let Some(path) = super::provider::find_executable(Path::new(executable), None) {
        add(path);
    }
    let mut homes = vec![
        PathBuf::from("/usr/lib/jvm"),
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        homes.push(PathBuf::from(home).join("Library/Java/JavaVirtualMachines"));
    }
    if let Some(home) = std::env::var_os("ProgramFiles") {
        homes.extend([
            PathBuf::from(&home).join("Java"),
            PathBuf::from(home).join("Eclipse Adoptium"),
        ]);
    }
    for home in homes {
        if let Ok(entries) = std::fs::read_dir(home) {
            let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
            entries.sort();
            for entry in entries {
                add(entry.join("bin").join(executable));
                add(entry.join("Contents/Home/bin").join(executable));
            }
        }
    }
    paths
}

fn validate(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_dir() {
        path.join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" })
    } else {
        path.to_owned()
    };
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot open Java executable {}: {e}", path.display()))?;
    if !path.is_file() {
        return Err("Choose a Java executable or a JDK home folder".into());
    }
    let mut cmd = Command::new(&path);
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Cannot run {} -version: {e}", path.display()))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(path),
            Ok(Some(status)) => {
                return Err(format!("{} -version failed: {status}", path.display()));
            }
            Err(e) => {
                super::kill_group(&mut child);
                let _ = child.wait();
                return Err(e.to_string());
            }
            _ if Instant::now() >= deadline => {
                super::kill_group(&mut child);
                let _ = child.wait();
                return Err("Java -version did not finish within 3 seconds".into());
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Change only the Java launcher argument; malformed or concurrently changed settings are left intact.
pub fn save(file: &Path, path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    let path = path.as_deref().map(validate).transpose()?;
    update_settings(file, "jdtls", "Java", |provider| {
        let args: Vec<String> = provider
            .get("args")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("jdtls args must be a list of strings: {e}"))?
            .unwrap_or_default();
        let mut kept = Vec::new();
        let mut args = args.into_iter().peekable();
        while let Some(arg) = args.next() {
            if arg == "--java-executable" {
                if args.peek().is_some_and(|value| !value.starts_with("--")) {
                    args.next();
                }
            } else if !arg.starts_with("--java-executable=") {
                kept.push(arg);
            }
        }
        if let Some(path) = &path {
            kept.push(format!(
                "--java-executable={}",
                path.to_str().ok_or("Java path must be valid UTF-8")?
            ));
        }
        provider.insert("args".into(), json!(kept));
        Ok(())
    })?;
    Ok(path)
}

/// Atomically change one provider while preserving unrelated and unknown settings.
pub(super) fn update_settings(
    file: &Path,
    provider_id: &str,
    label: &str,
    update: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), String>,
) -> Result<(), String> {
    let original = match std::fs::symlink_metadata(file) {
        Ok(_) => Some(
            lenz_content::theme::read_bounded(file)
                .map_err(|e| format!("Cannot read provider settings: {e}"))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Cannot read provider settings: {e}")),
    };
    let mut value: Value = original
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|e| format!("Provider settings contain invalid JSON: {e}"))?
        .unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or("Provider settings must be a JSON object")?;
    let providers = object
        .entry("providers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("providers must be a JSON object")?;
    let provider = providers
        .entry(provider_id)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("{provider_id} settings must be a JSON object"))?;
    update(provider)?;
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Provider settings exceed 64 KiB".into());
    }
    let parent = file
        .parent()
        .ok_or("Provider settings have no parent folder")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create provider settings folder: {e}"))?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = file.with_extension(format!(
        "runtime-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut output = options.open(&temp)?;
        if let Ok(metadata) = std::fs::metadata(file) {
            output.set_permissions(metadata.permissions())?;
        }
        output.write_all(&bytes)?;
        if lenz_content::theme::read_bounded(file).ok() != original {
            return Err(std::io::Error::other(
                "Provider settings changed; try again",
            ));
        }
        std::fs::rename(&temp, file)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(temp);
        return Err(format!("Could not save {label} runtime: {e}"));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("JDK with spaces/bin/java");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let temporary = path.with_extension("new");
        // Keep writable script descriptors out of children spawned by parallel tests.
        assert!(Command::new("/bin/sh")
            .args(["-c", "printf '%s' \"$1\" > \"$2\"", "fixture", &format!("#!/bin/sh\n{body}\n")])
            .arg(&temporary).status().unwrap().success());
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(temporary, &path).unwrap();
        path
    }

    #[test]
    fn runtime_save_rejects_concurrent_settings_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("providers.json");
        std::fs::write(&file, "{}").unwrap();
        let error = update_settings(&file, "pyright", "Node.js", |provider| {
            provider.insert("node".into(), json!("/node/bin/node"));
            std::fs::write(&file, r#"{"concurrent":true}"#).unwrap();
            Ok(())
        }).unwrap_err();
        assert!(error.contains("changed"), "{error}");
        assert_eq!(std::fs::read_to_string(file).unwrap(), r#"{"concurrent":true}"#);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn java_runtime_save_preserves_settings_and_reset_removes_only_the_override() {
        let dir = tempfile::tempdir().unwrap();
        let java = executable(dir.path(), "test \"$1\" = -version");
        let file = dir.path().join("providers.json");
        let mut expected = json!({"other":true,"providers":{"other":{"env":{"SECRET":"keep"}},"jdtls":{"disabled":true,"roots":{"a":"b"},"args":["--foo","value","--java-executable","old","--java-executable=older","--java-executable","--bar"],"settings":{"keep":42}}}});
        std::fs::write(&file, expected.to_string()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            save(&file, Some(java.parent().unwrap().parent().unwrap().into())).unwrap(),
            Some(java.clone())
        );
        expected["providers"]["jdtls"]["args"] = json!([
            "--foo",
            "value",
            "--bar",
            format!("--java-executable={}", java.display())
        ]);
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap(),
            expected
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let parsed =
            serde_json::from_value::<Vec<String>>(expected["providers"]["jdtls"]["args"].clone())
                .unwrap();
        assert_eq!(configured(&parsed), Some(java));
        save(&file, None).unwrap();
        expected["providers"]["jdtls"]["args"] = json!(["--foo", "value", "--bar"]);
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn java_runtime_invalid_input_and_malformed_settings_are_left_intact() {
        let dir = tempfile::tempdir().unwrap();
        let java = executable(dir.path(), "exit 0");
        let file = dir.path().join("providers.json");
        for text in ["{", "[]", r#"{"providers":{"jdtls":{"args":[1]}}}"#] {
            std::fs::write(&file, text).unwrap();
            assert!(save(&file, Some(java.clone())).is_err());
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        }
        std::fs::write(&file, "{}").unwrap();
        assert!(save(&file, Some(dir.path().join("missing"))).is_err());
        executable(dir.path(), "exit 13");
        assert!(save(&file, Some(java)).unwrap_err().contains("failed"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
        let blocked = dir.path().join("not-a-directory");
        std::fs::write(&blocked, "keep").unwrap();
        assert!(save(&blocked.join("providers.json"), None).is_err());
        assert_eq!(std::fs::read_to_string(blocked).unwrap(), "keep");
    }

    #[test]
    fn java_runtime_validation_times_out_without_changing_settings() {
        let dir = tempfile::tempdir().unwrap();
        let java = executable(dir.path(), "sleep 30");
        let file = dir.path().join("providers.json");
        std::fs::write(&file, "{}").unwrap();
        let start = Instant::now();
        let error = save(&file, Some(java)).unwrap_err();
        assert!(error.contains("3 seconds"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(6));
        assert_eq!(std::fs::read_to_string(file).unwrap(), "{}");
    }
}
