//! Explicit, curated language-server installs. Diagnostic text never becomes an executable command.
use std::{io::Read, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use super::provider::{Provider, Settings, find_executable};

pub const OUTPUT_LIMIT: usize = 32 * 1024;

/// Held from before the first subprocess through provider persistence, including across independent windows.
#[must_use = "Keep this lock alive until provider settings have been persisted"]
#[derive(Debug)]
pub struct InstallLock { _file: std::fs::File }


#[derive(Clone, Debug)]
pub struct Plan {
    pub provider: &'static str,
    pub manager: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub directory: PathBuf,
    pub executable: PathBuf,
    runtime: PathBuf,
    package: &'static str,
}

pub struct Preview {
    pub command: String,
    pub disabled: Option<String>,
    pub plan: Option<Plan>,
}

/// Current install dialog; only the core changes it. Dropping it cancels its owned worker.
pub struct Install {
    pub request: u64,
    pub provider: &'static str,
    pub preview: Preview,
    pub source: PathBuf,
    pub expected: Settings,
    pub running: bool,
    pub finished: bool,
    pub message: String,
    pub output: String,
    pub cancel: Arc<AtomicBool>,
}
impl Drop for Install {
    fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed); }
}
impl Install {
    pub fn text(&self) -> String {
        let mut text = format!("Install {}\n\n{}\n\n{}", self.provider, self.message, self.preview.command);
        if let Some(reason) = &self.preview.disabled { text.push_str(&format!("\n\n{reason}")); }
        if !self.output.is_empty() { text.push_str(&format!("\n\nInstaller output (up to 32 KiB; busy output may be omitted)\n{}", self.output)); }
        text
    }
}

/// Build argv from this allowlist alone. No global install, project manifests or shell evaluation.
pub fn preview(provider: &'static Provider, settings: &Settings, home: &Path) -> Preview {
    let package = match provider.id {
        "bash-language-server" => "bash-language-server@5.8.1",
        "pyright" => "pyright@1.1.414",
        "yaml-language-server" => "yaml-language-server@2.0.0",
        "typescript-language-server" => "typescript-language-server@6.0.1",
        "vscode-html" | "vscode-css" => "vscode-langservers-extracted@4.10.0",
        _ => return Preview { command: provider.setup.into(), disabled: Some("No managed installer is available for this server. Follow the setup instructions above, then Restart Language Server.".into()), plan: None },
    };
    let directory = home.join(provider.id);
    let mut args = vec!["install".into(), "--prefix".into(), directory.display().to_string(), "--no-audit".into(), "--no-fund".into(), "--package-lock=false".into(), "--ignore-scripts".into(), package.into()];
    if provider.id == "typescript-language-server" { args.push("typescript@6".into()); }
    let mut command = std::iter::once("npm".to_owned()).chain(args.iter().map(|s| quote(s))).collect::<Vec<_>>().join(" ");
    let launch = settings.launch(provider);
    let inherited = std::env::var_os("PATH");
    let path = launch.path().or(inherited.as_deref());
    let manager = find_executable(Path::new("npm"), path);
    let runtime = settings.providers.get(provider.id).and_then(|p| p.node.clone()).or_else(|| find_executable(Path::new("node"), path));
    let custom = settings.providers.get(provider.id).and_then(|s| s.executable.as_ref()).is_some_and(|p| *p != Path::new(provider.executable) && *p != directory.join("node_modules/.bin").join(provider.executable));
    let disabled = if !cfg!(unix) { Some("Managed installation currently requires Unix. Follow Server Details setup instructions.".into()) }
        else if custom { Some("A custom executable is configured. Repair that executable or remove its override in providers.json before using the managed installer.".into()) }
        else if manager.is_none() || runtime.as_ref().is_none_or(|p| !p.is_file()) { Some("Install Node.js and npm using your system's setup instructions, then reopen Install. Lenz does not install system package managers.".into()) }
        else if runtime.as_ref().is_some_and(|node| node.file_name() != Some(std::ffi::OsStr::new("node"))) { Some("The configured Node.js executable must be named node so the installed server can use the same runtime. Correct node or env.PATH in providers.json.".into()) }
        else { None };
    let plan = manager.zip(runtime).map(|(manager, runtime)| {
        command = std::iter::once(quote(&runtime.display().to_string())).chain(std::iter::once(quote(&manager.display().to_string()))).chain(args.iter().map(|s| quote(s))).collect::<Vec<_>>().join(" ");
        command.push_str(&format!("\nWorking directory: {}\nNode runtime: {}\nPackage scripts: disabled", directory.display(), runtime.display()));
        let mut env = launch.env;
        if let Some(bin) = runtime.parent() {
            let path = env.iter().find(|(k,_)| k == "PATH").map(|(_,v)| std::ffi::OsString::from(v)).or(inherited);
            if let Ok(path) = std::env::join_paths(std::iter::once(bin.to_owned()).chain(path.as_deref().into_iter().flat_map(std::env::split_paths))) {
                env.retain(|(key,_)| key != "PATH"); env.push(("PATH".into(), path.to_string_lossy().into_owned()));
            }
        }
        // Keep npm's cache and any package metadata in Lenz's own directory as well.
        env.push(("npm_config_cache".into(), directory.join("cache").display().to_string()));
        env.push(("npm_config_engine_strict".into(), "true".into()));
        Plan { provider: provider.id, manager, args, executable: directory.join("node_modules/.bin").join(provider.executable), directory, env, runtime, package }
    });
    Preview { command, disabled, plan }
}

fn quote(value: &str) -> String { format!("'{}'", value.replace('\'', "'\\''")) }

/// Stream a bounded amount of output while draining both pipes. Cancellation kills the complete owned group.
fn process(program: &Path, args: &[String], env: &[(String,String)], dir: &Path, cancel: &AtomicBool, timeout: Duration, output: &mut dyn FnMut(&str)) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { return Err("Installation cancelled".into()); }
    let mut command = Command::new(program);
    command.args(args).envs(env.iter().map(|(k,v)| (k,v))).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|why| format!("Cannot start {}: {why}", program.display()))?;
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    let mut readers = Vec::new();
    for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            let mut pipe = pipe;
            let mut buffer = [0; 2048];
            while let Ok(n) = pipe.read(&mut buffer) { if n == 0 || tx.send(buffer[..n].to_vec()).is_err() { break; } }
        }));
    }
    drop(tx);
    let (start, mut kept) = (Instant::now(), 0usize);
    let result = loop {
        for chunk in rx.try_iter().take(16) {
            let n = chunk.len().min(OUTPUT_LIMIT.saturating_sub(kept));
            if n > 0 { output(&String::from_utf8_lossy(&chunk[..n]).chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect::<String>()); kept += n; }
        }
        if cancel.load(Ordering::Relaxed) { break Err("Installation cancelled".into()); }
        if start.elapsed() > timeout { break Err("Installer timed out".into()); }
        match child.try_wait() {
            Ok(Some(status)) => break if status.success() { Ok(()) } else { Err(format!("Installer exited with {status}")) },
            Err(why) => break Err(format!("Cannot wait for installer: {why}")),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    super::kill_group(&mut child); let _ = child.wait();
    // A child may leave descendants holding pipes; killing its group closes those too.
    for chunk in rx.iter() {
        let n = chunk.len().min(OUTPUT_LIMIT.saturating_sub(kept));
        if n > 0 { output(&String::from_utf8_lossy(&chunk[..n]).chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect::<String>()); kept += n; }
    }
    for reader in readers { let _ = reader.join(); }
    result
}

/// Execute only a previously previewed recipe. Keep the returned lock until the validated launch is persisted.
pub fn run(plan: &Plan, cancel: &AtomicBool, output: &mut dyn FnMut(&str)) -> Result<InstallLock, String> {
    std::fs::create_dir_all(&plan.directory).map_err(|e| format!("Cannot create managed server directory: {e}"))?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(plan.directory.join(".lenz-install.lock")).map_err(|e|format!("Cannot open installer lock: {e}"))?;
    file.try_lock().map_err(|e| match e {
        std::fs::TryLockError::WouldBlock => "An installation is already running for this provider in another Lenz window or process".to_owned(),
        std::fs::TryLockError::Error(e) => format!("Cannot lock managed server directory: {e}"),
    })?;
    let lock = InstallLock { _file: file };
    output("Checking Node.js runtime…\n");
    let mut version = String::new();
    process(&plan.runtime, &["--version".into()], &plan.env, &plan.directory, cancel, Duration::from_secs(10), &mut |chunk| { version.push_str(chunk); output(chunk); })?;
    let version = version.trim().strip_prefix('v').ok_or("Selected runtime did not report a Node.js version")?;
    if version.split('.').count() != 3 || !version.split('.').all(|part| part.parse::<u32>().is_ok()) { return Err("Selected runtime did not report a Node.js version".into()); }
    output("Downloading and installing the selected npm package…\n");
    // Run npm with the selected Node explicitly, without shebang or shell-startup fallback.
    let args = std::iter::once(plan.manager.display().to_string()).chain(plan.args.iter().cloned()).collect::<Vec<_>>();
    process(&plan.runtime, &args, &plan.env, &plan.directory, cancel, Duration::from_secs(15 * 60), output)
        .map_err(|why| format!("{why}. npm must be its JavaScript CLI, usable with the selected Node.js runtime"))?;
    let canonical = std::fs::canonicalize(&plan.executable).map_err(|_| "Installed server executable was not created")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&canonical).map_err(|_| "Cannot inspect installed executable")?.permissions().mode() & 0o111 == 0 { return Err("Installed server is not executable".into()); }
    }
    let root = std::fs::canonicalize(&plan.directory).map_err(|_| "Managed server directory disappeared")?;
    if !canonical.starts_with(root) { return Err("Installed executable points outside its managed directory".into()); }
    let (package, expected) = plan.package.split_once('@').ok_or("Invalid curated package version")?;
    if super::provider::package_version(&plan.executable, package).as_deref() != Some(expected) { return Err("Installed server package identity/version failed validation".into()); }
    // Validate with Node's syntax checker: no protocol session and no workspace/project code is run.
    output("Validating installed server…\n");
    process(&plan.runtime, &["--check".into(), canonical.display().to_string()], &plan.env, &plan.directory, cancel, Duration::from_secs(10), output)?;
    Ok(lock)
}

/// Merge only the validated executable/runtime into the latest file. Refuse a changed target provider.
pub fn persist(plan: &Plan, file: &Path, expected: &Settings, cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { return Err("Installation cancelled; provider settings were not changed".into()); }
    super::provider::update_settings(file, plan.provider, "installed language server", |provider| {
        let current: super::provider::ProviderSettings = serde_json::from_value(serde_json::Value::Object(provider.clone()))
            .map_err(|e| format!("Invalid provider settings: {e}"))?;
        if current != expected.providers.get(plan.provider).cloned().unwrap_or_default() {
            return Err("Provider settings changed during installation; installed files were kept, settings were not replaced".into());
        }
        if cancel.load(Ordering::Relaxed) { return Err("Installation cancelled; provider settings were not changed".into()); }
        provider.insert("executable".into(), serde_json::json!(plan.executable));
        let env = provider.entry("env").or_insert_with(|| serde_json::json!({})).as_object_mut().ok_or("Provider env must be an object")?;
        if let Some((_, path)) = plan.env.iter().find(|(key,_)| key == "PATH") { env.insert("PATH".into(), serde_json::json!(path)); }
        if plan.provider == "pyright" && !provider.contains_key("node") { provider.insert("node".into(), serde_json::json!(plan.runtime)); }
        Ok(())
    })
}
