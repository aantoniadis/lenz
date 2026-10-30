use std::{ffi::OsString, path::PathBuf, process::ExitCode};

const USAGE: &str = "Usage: lenz-code [file-or-folder]
       lenz-code -- <file-or-folder>   (a name starting with -)
       lenz-code --help | --version
lenz takes the same arguments.";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if lenz_content::about::wants_version(&args) {
        println!("{}", lenz_content::about::version_line("lenz-code"));
        return ExitCode::SUCCESS;
    }
    // An option is never a file to open: only `--` makes the next argument one.
    let path = match args.as_slice() {
        [] => None,
        [a] if a == "--help" || a == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [dash, path] if dash == "--" => Some(PathBuf::from(path)),
        [a] if !a.to_string_lossy().starts_with('-') => Some(PathBuf::from(a)),
        [a] => return usage(&format!("unknown option {}", a.to_string_lossy())),
        _ => return usage("one file or folder at most"),
    };
    code_gui::run(path);
    ExitCode::SUCCESS
}

fn usage(why: &str) -> ExitCode {
    eprintln!("lenz-code: {why}\n{USAGE}");
    ExitCode::from(2)
}
