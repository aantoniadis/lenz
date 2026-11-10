//! Managed server recipes using an existing Ruby or Go toolchain.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Toolchain { Node, Ruby, Go }

const RUBY_VERSION: &str = "0.26.11";
const GOPLS_VERSION: &str = "v0.24.0";

pub(super) fn preview(provider: &'static Provider, settings: &Settings, home: &Path) -> Option<Preview> {
    let (toolchain, runtime_name, manager_name, package) = match provider.id {
        "ruby-lsp" => (Toolchain::Ruby, "ruby", "gem", "ruby-lsp@0.26.11"),
        "gopls" => (Toolchain::Go, "go", "go", "gopls@v0.24.0"),
        _ => return None,
    };
    let directory = home.join(provider.id);
    let executable = directory.join("bin").join(provider.executable);
    let launch = settings.launch(provider);
    let inherited = std::env::var_os("PATH");
    let path = launch.path().or(inherited.as_deref());
    let runtime = find_executable(Path::new(runtime_name), path);
    let manager = find_executable(Path::new(manager_name), path);
    let custom = settings.providers.get(provider.id).and_then(|p| p.executable.as_ref())
        .is_some_and(|p| p != Path::new(provider.executable) && *p != executable);
    let disabled = if !cfg!(unix) { Some("Managed installation currently requires Unix. Follow Server Details setup instructions.".into()) }
        else if custom { Some("A custom executable is configured. Repair it or remove its override in providers.json before using managed installation.".into()) }
        else if runtime.is_none() || manager.is_none() { Some(format!("Install {runtime_name} and {manager_name} first, then reopen Install Language Server. Use the same runtime as your project; Lenz does not install system toolchains.")) }
        else { None };
    let args = match toolchain {
        Toolchain::Ruby => vec!["install".into(), "ruby-lsp".into(), "--norc".into(), "--version".into(), RUBY_VERSION.into(), "--install-dir".into(), directory.join("gems").display().to_string(), "--bindir".into(), directory.join("bin").display().to_string(), "--no-document".into(), "--env-shebang".into()],
        Toolchain::Go => vec!["install".into(), format!("golang.org/x/tools/gopls@{GOPLS_VERSION}")],
        Toolchain::Node => return None,
    };
    let mut command = std::iter::once(manager.as_ref().map_or_else(|| manager_name.into(), |p| quote(&p.display().to_string())))
        .chain(args.iter().map(|a| quote(a))).collect::<Vec<_>>().join(" ");
    command.push_str(&format!("\nDestination: {}\nRequires existing {runtime_name}. Downloads dependencies and may compile native code. No sudo or project manifest changes.\n\n{}", directory.display(), provider.setup));
    let plan = runtime.zip(manager).map(|(runtime, manager)| {
        let mut env = launch.env;
        if let Some(bin) = runtime.parent() {
            let inherited = env.iter().find(|(k,_)| k == "PATH").map(|(_,v)| std::ffi::OsString::from(v)).or(inherited);
            if let Ok(path) = std::env::join_paths(std::iter::once(bin.to_owned()).chain(inherited.as_deref().into_iter().flat_map(std::env::split_paths))) {
                env.retain(|(k,_)| k != "PATH"); env.push(("PATH".into(), path.to_string_lossy().into_owned()));
            }
        }
        let mut set = |key: &str, value: String| { env.retain(|(k,_)| k != key); env.push((key.into(), value)); };
        match toolchain {
            Toolchain::Ruby => {
                let gems = directory.join("gems");
                set("GEM_HOME", gems.display().to_string());
                set("GEM_SPEC_CACHE", directory.join("spec-cache").display().to_string());
                let inherited = settings.providers.get(provider.id).and_then(|p| p.env.get("GEM_PATH")).cloned().or_else(|| std::env::var("GEM_PATH").ok());
                if let Some(inherited) = inherited
                    && let Ok(path) = std::env::join_paths(std::iter::once(gems).chain(std::env::split_paths(&inherited))) {
                    set("GEM_PATH", path.to_string_lossy().into_owned());
                }
            }
            Toolchain::Go => {
                for (key, value) in [("GOBIN", directory.join("bin")), ("GOMODCACHE", directory.join("modules")), ("GOCACHE", directory.join("cache"))] { set(key, value.display().to_string()); }
                set("GOWORK", "off".into()); set("GOTOOLCHAIN", "local".into()); set("GOFLAGS", String::new());
            }
            Toolchain::Node => {}
        }
        Plan { provider: provider.id, manager, args, env, directory, executable, runtime, package, toolchain }
    });
    Some(Preview { command, disabled, plan })
}

pub(super) fn run(plan: &Plan, cancel: &AtomicBool, output: &mut dyn FnMut(&str)) -> Result<(), String> {
    output("Checking the selected runtime…\n");
    let version_arg = if plan.toolchain == Toolchain::Go { "version" } else { "--version" };
    let mut version = String::new();
    process(&plan.runtime, &[version_arg.into()], &plan.env, &plan.directory, cancel, Duration::from_secs(10), &mut |chunk| { version.push_str(chunk); output(chunk); })?;
    let expected = if plan.toolchain == Toolchain::Go { "go version go" } else { "ruby " };
    if !version.trim_start().starts_with(expected) { return Err("Selected runtime did not report the expected toolchain version".into()); }
    output("Downloading and installing the selected server…\n");
    process(&plan.manager, &plan.args, &plan.env, &plan.directory, cancel, Duration::from_secs(15 * 60), output)?;
    let executable = std::fs::canonicalize(&plan.executable).map_err(|_| "Installed server executable was not created")?;
    let root = std::fs::canonicalize(&plan.directory).map_err(|_| "Managed directory disappeared")?;
    if !executable.starts_with(&root) { return Err("Installed executable points outside its managed directory".into()); }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&executable).map_err(|_| "Cannot inspect installed server")?.permissions().mode() & 0o111 == 0 { return Err("Installed server is not executable".into()); }
    }
    output("Validating the installed server…\n");
    let mut found = String::new();
    match plan.toolchain {
        Toolchain::Ruby => {
            process(&plan.runtime, &["--disable=gems".into(), "-c".into(), executable.display().to_string()], &plan.env, &plan.directory, cancel, Duration::from_secs(10), output)?;
            // Resolve the pinned gem through the exact environment saved for the server.
            let code = format!("spec = Gem::Specification.find_by_name('ruby-lsp', '= {RUBY_VERSION}'); abort('Gem is outside its managed directory') unless File.realpath(spec.full_gem_path).start_with?(ARGV.fetch(0) + File::SEPARATOR); puts spec.version");
            let gems = std::fs::canonicalize(plan.directory.join("gems")).map_err(|_| "Installed gem directory is missing")?;
            process(&plan.runtime, &["-rrubygems".into(), "-e".into(), code, gems.display().to_string()], &plan.env, &plan.directory, cancel, Duration::from_secs(10), &mut |chunk| found.push_str(chunk))?;
            if found.trim() != RUBY_VERSION { return Err("Installed Ruby LSP version failed validation".into()); }
        }
        Toolchain::Go => {
            process(&executable, &["version".into()], &plan.env, &plan.directory, cancel, Duration::from_secs(10), &mut |chunk| found.push_str(chunk))?;
            if !found.lines().any(|line| line.split_whitespace().collect::<Vec<_>>() == ["golang.org/x/tools/gopls", GOPLS_VERSION]) { return Err("Installed gopls version failed validation".into()); }
        }
        Toolchain::Node => return Err("Incorrect installer toolchain".into()),
    }
    Ok(())
}
