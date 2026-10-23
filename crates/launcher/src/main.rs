#![forbid(unsafe_code)]
use std::{env, ffi::OsString, io, os::unix::process::CommandExt, process::{Command, ExitCode}};

const HELP: &str = "Lenz — code and comparisons

Usage: lenz [file-or-folder]
       lenz --tui [file-or-folder]
       lenz --version
       lenz diff [comparison arguments]
       lenz diff --tui [comparison arguments]

Use lenz diff --help for comparison options.
With no path, lenz opens the code reader in the current directory.
Use lenz diff log and lenz diff merge for repository and merge workflows.
Existing difftool/mergetool commands must add the diff prefix, or use lenz-diff directly.
To read a file or folder named diff, use lenz ./diff.
After --, all arguments are passed unchanged (including a file named --tui).";

fn main() -> ExitCode {
    let mut args: Vec<OsString> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|s| s == "--help" || s == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    // The launcher's own: no app starts. (No dependencies here, so the build ID is read as the apps read it.)
    if matches!(args.as_slice(), [a] if a == "--version" || a == "-V") {
        println!("lenz {}", option_env!("LENZ_BUILD_ID").unwrap_or(env!("CARGO_PKG_VERSION")));
        return ExitCode::SUCCESS;
    }
    let binary = match args.first().and_then(|s| s.to_str()) {
        Some("--tui") => {
            args.remove(0);
            "lenz-code-tui"
        }
        Some("diff") => {
            args.remove(0);
            if args.first().is_some_and(|s| s == "--tui") {
                args.remove(0);
                "lenz-diff-tui"
            } else { "lenz-diff" }
        }
        _ => "lenz-code",
    };
    // The apps sit beside the real launcher: `lenz` on PATH may be a symlink to it (Homebrew, the macOS bundle), and
    // macOS reports the symlink's path.
    let error = match env::current_exe().map(|path| std::fs::canonicalize(&path).unwrap_or(path)) {
        Ok(mut path) => { path.set_file_name(binary); Command::new(path).args(args).exec() }
        Err(error) => error,
    };
    eprintln!("lenz: cannot start {binary}: {error}. Keep the Lenz executables together: reinstall Lenz, or rebuild with scripts/try-launcher.sh.");
    ExitCode::from(if error.kind() == io::ErrorKind::NotFound { 127 } else { 126 })
}
