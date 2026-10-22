//! Open in another app, shared by Lenz Diff and Lenz Code: a file to the user's editor at the line, or to the
//! system default. Resolution is a pure function of a [`Host`] (env vars, PATH, platform); the UI spawns the [`Launch`].


use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The toast when `o` finds no editor and falls back to the system default.
pub const NO_EDITOR_TOAST: &str = "Set $VISUAL (or LENZ_EDITOR) to open at the line";

/// A process to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// What [`plan`] decided: the launch, and a toast to show with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub launch: Launch,
    pub toast: Option<String>,
}

/// The file to open: an absolute path (the working-tree file or a [`copy`]), where the caret is (1-based), and
/// whether its bytes are binary (`SideBytes::binary`).
pub struct Target<'a> {
    pub file: &'a Path,
    pub binary: bool,
    pub line: Option<usize>,
    pub column: Option<usize>,
    /// Where the editor or terminal starts, e.g. the repository's root.
    pub cwd: Option<&'a Path>,
}

pub type Lookup<T> = Box<dyn Fn(&str) -> T>;

/// The outside world resolution reads: env vars, which programs are on PATH, and the platform.
pub struct Host {
    pub var: Lookup<Option<String>>,
    pub on_path: Lookup<bool>,
    pub macos: bool,
}

impl Host {
    pub fn current() -> Host {
        Host { var: Box::new(|k| std::env::var(k).ok()), on_path: Box::new(on_path), macos: cfg!(target_os = "macos") }
    }

    /// A set, non-blank variable.
    pub fn get(&self, key: &str) -> Option<String> {
        (self.var)(key).filter(|v| !v.trim().is_empty())
    }
}

fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

/// Opens `target` in the editor at its line (`default_app` false) or with the system default (true), with no
/// settings-file editor.
pub fn plan(host: &Host, target: &Target, default_app: bool) -> Plan {
    plan_with(host, target, None, default_app)
}

/// [`plan`] with `configured` as the settings file's editor, after the environment's.
pub fn plan_with(host: &Host, target: &Target, configured: Option<&str>, default_app: bool) -> Plan {
    let cwd = target.cwd.map(Path::to_path_buf);
    let file = target.file.to_string_lossy().into_owned(); // ponytail: non-UTF-8 paths go lossy; OsString args if one turns up
    let system = |toast: Option<String>| {
        // gio wraps a terminal default app (e.g. nvim.desktop) in a terminal; xdg-open would start it without one
        let (program, args) = if host.macos { ("open", vec![file.clone()]) }
            else if (host.on_path)("gio") { ("gio", vec!["open".into(), file.clone()]) }
            else { ("xdg-open", vec![file.clone()]) };
        Plan { launch: Launch { program: program.into(), args, cwd: cwd.clone() }, toast }
    };
    if default_app || !opens_in_editor(target.file, target.binary) {
        return system(None);
    }
    let Some(editor) = editor_with(host, configured) else { return system(Some(NO_EDITOR_TOAST.into())) };
    let (argv, in_terminal) = editor_argv(&editor, &file, target.line, target.column);
    // $EDITOR names a terminal editor by convention ($VISUAL a GUI one), so an unrecognised wrapper from it
    // (`omarchy-launch-editor --inline`) gets a terminal instead of running invisibly
    let from_editor = ["LENZ_EDITOR", "VISUAL"].iter().all(|k| host.get(k).is_none()) && host.get("EDITOR").is_some();
    let in_terminal = in_terminal || from_editor && !gui_editor(&editor[0]);
    // editor_argv keeps the editor's program first
    if !in_terminal && let Some((program, args)) = argv.split_first() {
        return Plan { launch: Launch { program: program.clone(), args: args.to_vec(), cwd }, toast: None };
    }
    match terminal(host, &argv, target.cwd) {
        Some(launch) => Plan { launch: Launch { cwd, ..launch }, toast: None },
        None => system(Some(format!("No terminal found to run {}; set $TERMINAL", editor[0]))),
    }
}

/// Text goes to the editor, except web pages and SVGs, which read better in a browser. Binary goes to the system.
pub fn opens_in_editor(file: &Path, binary: bool) -> bool {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    !binary && !matches!(ext.as_str(), "html" | "htm" | "svg")
}

/// `LENZ_EDITOR`, else `$VISUAL`, else `$EDITOR`, else `configured` (a settings file's `editor`), split into
/// program and arguments.
pub fn editor_with(host: &Host, configured: Option<&str>) -> Option<Vec<String>> {
    ["LENZ_EDITOR", "VISUAL", "EDITOR"].iter().filter_map(|k| host.get(k)).chain(configured.map(str::to_string))
        .map(|v| split(&v)).find(|argv| !argv.is_empty())
}

/// Shell-like word splitting without a shell: whitespace separates, `'…'` is literal, `"…"` honours `\"` and `\\`,
/// a backslash outside quotes escapes the next character. An unterminated quote runs to the end.
pub fn split(s: &str) -> Vec<String> {
    let (mut words, mut word, mut in_word) = (Vec::new(), String::new(), false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                continue;
            }
            '\'' => word.extend(chars.by_ref().take_while(|&c| c != '\'')),
            '"' => {
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some(e @ ('"' | '\\')) => word.push(e),
                            Some(e) => word.extend(['\\', e]),
                            None => word.push('\\'),
                        },
                        c => word.push(c),
                    }
                }
            }
            '\\' => word.extend(chars.next()),
            c => word.push(c),
        }
        in_word = true;
    }
    if in_word {
        words.push(word);
    }
    words
}

fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// The editor's argv for `file` at `line`/`column`, and whether it must run in a terminal.
/// Unknown editors (wrappers included) get the file alone.
pub fn editor_argv(editor: &[String], file: &str, line: Option<usize>, column: Option<usize>) -> (Vec<String>, bool) {
    let name = basename(&editor[0]);
    let name = name.strip_suffix(".sh").unwrap_or(name); // JetBrains' `idea.sh`
    let mut argv = editor.to_vec();
    let at = |sep: &str| match (line, column) {
        (Some(l), Some(c)) => format!("{file}{sep}{l}:{c}"),
        (Some(l), None) => format!("{file}{sep}{l}"),
        _ => file.to_string(),
    };
    let in_term = match name {
        "nvim" | "vim" | "vi" | "nano" | "micro" | "kak" | "kakoune" => true,
        "emacs" | "emacsclient" => editor[1..].iter().any(|a| matches!(a.as_str(), "-nw" | "-t" | "--no-window-system" | "--tty")),
        "hx" | "helix" => true,
        _ => false,
    };
    match name {
        "nvim" | "vim" | "vi" | "nano" | "micro" | "kak" | "kakoune" | "emacs" | "emacsclient" => {
            argv.extend(line.map(|l| format!("+{l}")));
            argv.push(file.to_string());
        }
        "hx" | "helix" => argv.push(at(":")),
        "code" | "code-insiders" | "codium" | "vscodium" => match line {
            Some(_) => argv.extend(["--goto".to_string(), at(":")]),
            None => argv.push(file.to_string()),
        },
        "zed" | "zeditor" | "subl" => argv.push(at(":")),
        "idea" | "clion" | "goland" | "pycharm" | "rustrover" | "webstorm" | "phpstorm" | "rubymine" | "rider" | "datagrip"
        | "dataspell" => {
            if let Some(l) = line {
                argv.extend(["--line".to_string(), l.to_string()]);
                if let Some(c) = column {
                    argv.extend(["--column".to_string(), c.to_string()]);
                }
            }
            argv.push(file.to_string());
        }
        _ => argv.push(file.to_string()),
    }
    (argv, in_term)
}

/// Editors that open their own window, whichever variable names them.
fn gui_editor(program: &str) -> bool {
    let name = basename(program);
    matches!(name.strip_suffix(".sh").unwrap_or(name),
        "code" | "code-insiders" | "codium" | "vscodium" | "zed" | "zeditor" | "subl" | "idea" | "clion" | "goland" | "pycharm"
        | "rustrover" | "webstorm" | "phpstorm" | "rubymine" | "rider" | "datagrip" | "dataspell" | "gedit" | "gnome-text-editor"
        | "kate" | "kwrite" | "gvim" | "neovide" | "mousepad" | "geany" | "xed" | "pluma" | "emacs" | "emacsclient")
}

/// A terminal window running `command`: Linux tries `$TERMINAL`, `xdg-terminal-exec`, then known terminals on PATH;
/// macOS uses `$TERMINAL` when set, else Terminal.app. `None` when Linux has no terminal at all.
pub fn terminal(host: &Host, command: &[String], cwd: Option<&Path>) -> Option<Launch> {
    if let Some(term) = host.get("TERMINAL").map(|t| split(&t)).filter(|t| !t.is_empty()) {
        return Some(in_terminal(&term, command));
    }
    if host.macos {
        return Some(terminal_app(command, cwd));
    }
    ["xdg-terminal-exec", "ghostty", "kitty", "alacritty", "foot", "wezterm", "gnome-terminal", "konsole"]
        .into_iter()
        .find(|t| (host.on_path)(t))
        .map(|t| in_terminal(&[t.to_string()], command))
}

/// `term` (program and its own arguments) running `command`, with that terminal's syntax; unknown ones get `-e`.
fn in_terminal(term: &[String], command: &[String]) -> Launch {
    let run: &[&str] = match basename(&term[0]) {
        "xdg-terminal-exec" | "foot" => &[],
        "kitty" | "gnome-terminal" => &["--"],
        "wezterm" => &["start", "--"],
        _ => &["-e"], // alacritty, ghostty, konsole, xterm and most others
    };
    let args = term[1..].iter().cloned().chain(run.iter().map(|s| s.to_string())).chain(command.iter().cloned());
    Launch { program: term[0].clone(), args: args.collect(), cwd: None }
}

/// Terminal.app running `command` through `osascript`: shell-quoted, then escaped into an AppleScript string.
fn terminal_app(command: &[String], cwd: Option<&Path>) -> Launch {
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let mut shell = command.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    if let Some(dir) = cwd {
        shell = format!("cd {} && {shell}", quote(&dir.to_string_lossy()));
    }
    let script = shell.replace('\\', r"\\").replace('"', r#"\""#);
    let args = ["-e", &format!(r#"tell application "Terminal" to do script "{script}""#), "-e", r#"tell application "Terminal" to activate"#];
    Launch { program: "osascript".into(), args: args.map(String::from).to_vec(), cwd: None }
}

/// Starts `launch` detached: no stdio, its own process group, reaped by a thread.
pub fn spawn(launch: &Launch) -> io::Result<()> {
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(dir) = &launch.cwd {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Whether `url` is an `http(s)://` link, the only kind Lenz opens or shows as a link.
pub fn is_web_link(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Opens a web link in the default browser. Links come from untrusted text (PR bodies, comments), so only
/// `http(s)://` is accepted: `file://`, custom scheme handlers and option-like values never reach `xdg-open`.
pub fn browse(url: &str) -> Result<(), String> {
    if !is_web_link(url) {
        return Err(format!("Only web links open here: {url}"));
    }
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    spawn(&Launch { program: program.into(), args: vec![url.into()], cwd: None }).map_err(|e| format!("Cannot open the browser: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_links_reach_the_browser() {
        for bad in ["file:///etc/passwd", "smb://host/share", "vscode://x", "-n", "javascript:alert(1)", "", "http:/x"] {
            assert!(browse(bad).unwrap_err().starts_with("Only web links"), "{bad}");
        }
    }

    use super::*;
    use std::collections::HashMap;

    fn host(vars: &[(&str, &str)], path: &[&str], macos: bool) -> Host {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let path: Vec<String> = path.iter().map(|p| p.to_string()).collect();
        Host { var: Box::new(move |k| vars.get(k).cloned()), on_path: Box::new(move |p| path.iter().any(|q| q == p)), macos }
    }

    fn target(file: &str) -> Target<'_> {
        Target { file: Path::new(file), binary: false, line: Some(12), column: Some(5), cwd: None }
    }

    fn launch(program: &str, args: &[&str]) -> Launch {
        Launch { program: program.into(), args: args.iter().map(|a| a.to_string()).collect(), cwd: None }
    }

    fn argv(editor: &str, line: Option<usize>, column: Option<usize>) -> (Vec<String>, bool) {
        editor_argv(&split(editor), "/r/a.rs", line, column)
    }

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn text_goes_to_the_editor_web_pages_and_binaries_to_the_system() {
        assert!(opens_in_editor(Path::new("src/main.rs"), false));
        assert!(opens_in_editor(Path::new("Makefile"), false));
        assert!(!opens_in_editor(Path::new("index.html"), false));
        assert!(!opens_in_editor(Path::new("a/b.HTM"), false));
        assert!(!opens_in_editor(Path::new("logo.svg"), false));
        assert!(!opens_in_editor(Path::new("photo.png"), true));
        assert!(!opens_in_editor(Path::new("doc.pdf"), true));
    }

    #[test]
    fn editor_precedence_skips_unset_and_blank_variables() {
        let all = [("LENZ_EDITOR", "nvim"), ("VISUAL", "code --wait"), ("EDITOR", "vi")];
        assert_eq!(editor_with(&host(&all, &[], false), None), Some(words(&["nvim"])));
        assert_eq!(editor_with(&host(&all[1..], &[], false), None), Some(words(&["code", "--wait"])));
        assert_eq!(editor_with(&host(&[("VISUAL", ""), ("EDITOR", "vi")], &[], false), None), Some(words(&["vi"])));
        assert_eq!(editor_with(&host(&[("LENZ_EDITOR", "  "), ("EDITOR", "vi")], &[], false), None), Some(words(&["vi"])));
        assert_eq!(editor_with(&host(&[("VISUAL", "")], &[], false), None), None);
    }

    #[test]
    fn the_settings_editor_comes_after_the_environment() {
        let h = host(&[], &[], false);
        assert_eq!(editor_with(&h, Some("hx")), Some(words(&["hx"])));
        let h = host(&[("EDITOR", "vi")], &[], false);
        assert_eq!(editor_with(&h, Some("hx")), Some(words(&["vi"])));
        assert_eq!(editor_with(&host(&[], &[], false), None), None);
    }

    #[test]
    fn splitting_follows_simple_shell_quoting() {
        assert_eq!(split("code --wait"), words(&["code", "--wait"]));
        assert_eq!(split("  emacs   -nw "), words(&["emacs", "-nw"]));
        assert_eq!(split(r#""/Applications/Sublime Text.app/bin/subl" -w"#), words(&["/Applications/Sublime Text.app/bin/subl", "-w"]));
        assert_eq!(split("'my editor' -x"), words(&["my editor", "-x"]));
        assert_eq!(split(r#"a"b c"d"#), words(&["ab cd"]));
        assert_eq!(split(r#""say \"hi\" \\ \n""#), words(&[r#"say "hi" \ \n"#]));
        assert_eq!(split(r"my\ editor ''"), words(&["my editor", ""]));
        assert_eq!(split("'unterminated arg"), words(&["unterminated arg"]));
        assert_eq!(split(""), Vec::<String>::new());
    }

    #[test]
    fn plus_line_editors() {
        for (editor, terminal) in [("nvim", true), ("vim", true), ("vi", true), ("nano", true), ("micro", true), ("kak", true), ("kakoune", true), ("emacs", false)] {
            assert_eq!(argv(editor, Some(12), Some(5)), (words(&[editor, "+12", "/r/a.rs"]), terminal), "{editor}");
        }
        assert_eq!(argv("/usr/bin/nvim -u NONE", Some(3), None), (words(&["/usr/bin/nvim", "-u", "NONE", "+3", "/r/a.rs"]), true));
        assert_eq!(argv("emacs -nw", Some(3), None), (words(&["emacs", "-nw", "+3", "/r/a.rs"]), true));
        assert_eq!(argv("emacsclient -t", Some(3), None), (words(&["emacsclient", "-t", "+3", "/r/a.rs"]), true));
        assert_eq!(argv("nvim", None, None), (words(&["nvim", "/r/a.rs"]), true));
    }

    #[test]
    fn helix_takes_file_colon_line_and_column() {
        assert_eq!(argv("hx", Some(12), Some(5)), (words(&["hx", "/r/a.rs:12:5"]), true));
        assert_eq!(argv("helix", Some(12), None), (words(&["helix", "/r/a.rs:12"]), true));
    }

    #[test]
    fn gui_editors_get_their_own_line_syntax() {
        assert_eq!(argv("code --wait", Some(12), Some(5)), (words(&["code", "--wait", "--goto", "/r/a.rs:12:5"]), false));
        assert_eq!(argv("codium", None, None), (words(&["codium", "/r/a.rs"]), false));
        assert_eq!(argv("code-insiders", Some(1), None), (words(&["code-insiders", "--goto", "/r/a.rs:1"]), false));
        assert_eq!(argv("zed", Some(12), Some(5)), (words(&["zed", "/r/a.rs:12:5"]), false));
        assert_eq!(argv("zeditor", Some(12), Some(5)), (words(&["zeditor", "/r/a.rs:12:5"]), false));
        assert_eq!(argv("subl", Some(12), Some(5)), (words(&["subl", "/r/a.rs:12:5"]), false));
        for ide in ["idea", "idea.sh", "clion", "goland", "pycharm", "rustrover", "webstorm"] {
            assert_eq!(argv(ide, Some(12), Some(5)), (words(&[ide, "--line", "12", "--column", "5", "/r/a.rs"]), false), "{ide}");
        }
        assert_eq!(argv("idea", Some(12), None), (words(&["idea", "--line", "12", "/r/a.rs"]), false));
    }

    #[test]
    fn unknown_editors_and_wrappers_get_the_file_alone() {
        assert_eq!(argv("omarchy-launch-editor --inline", Some(12), Some(5)), (words(&["omarchy-launch-editor", "--inline", "/r/a.rs"]), false));
    }

    #[test]
    fn linux_terminal_chain_and_per_terminal_syntax() {
        let cmd = words(&["nvim", "+3", "/r/a.rs"]);
        let expect = |program: &str, run: &[&str]| {
            let mut args: Vec<&str> = run.to_vec();
            args.extend(["nvim", "+3", "/r/a.rs"]);
            Some(launch(program, &args))
        };
        // $TERMINAL wins, through the known table (xdg-terminal-exec takes the command bare)
        let all = ["xdg-terminal-exec", "ghostty", "kitty"];
        assert_eq!(terminal(&host(&[("TERMINAL", "xdg-terminal-exec")], &[], false), &cmd, None), expect("xdg-terminal-exec", &[]));
        assert_eq!(terminal(&host(&[("TERMINAL", "st")], &all, false), &cmd, None), expect("st", &["-e"]));
        assert_eq!(terminal(&host(&[("TERMINAL", "/usr/bin/foot")], &[], false), &cmd, None), expect("/usr/bin/foot", &[]));
        assert_eq!(terminal(&host(&[("TERMINAL", "kitty --single-instance")], &[], false), &cmd, None), expect("kitty", &["--single-instance", "--"]));
        // then the first on PATH, in order
        assert_eq!(terminal(&host(&[("TERMINAL", "")], &all, false), &cmd, None), expect("xdg-terminal-exec", &[]));
        for (term, run) in [
            ("ghostty", &["-e"][..]),
            ("kitty", &["--"]),
            ("alacritty", &["-e"]),
            ("foot", &[]),
            ("wezterm", &["start", "--"]),
            ("gnome-terminal", &["--"]),
            ("konsole", &["-e"]),
        ] {
            assert_eq!(terminal(&host(&[], &[term, "konsole"], false), &cmd, None), expect(term, run), "{term}");
        }
        assert_eq!(terminal(&host(&[], &["xterm"], false), &cmd, None), None);
    }

    #[test]
    fn macos_uses_terminal_app_with_quoted_command() {
        let cmd = words(&["nvim", "+3", r#"/Users/a/it's "q" \ x.rs"#]);
        let got = terminal(&host(&[], &["kitty"], true), &cmd, Some(Path::new("/Users/a/my repo"))).unwrap();
        let script = r#"tell application "Terminal" to do script "cd '/Users/a/my repo' && 'nvim' '+3' '/Users/a/it'\\''s \"q\" \\ x.rs'""#;
        assert_eq!(got, launch("osascript", &["-e", script, "-e", r#"tell application "Terminal" to activate"#]));
        // $TERMINAL still wins on macOS
        assert_eq!(terminal(&host(&[("TERMINAL", "alacritty")], &[], true), &cmd, None).unwrap().program, "alacritty");
    }

    #[test]
    fn plan_routes_o_and_shift_o() {
        let cwd = Path::new("/r");
        let with_cwd = |l: Launch| Launch { cwd: Some(cwd.to_path_buf()), ..l };
        let t = Target { cwd: Some(cwd), ..target("/r/a.rs") };
        let code = host(&[("VISUAL", "code")], &[], false);
        assert_eq!(plan(&code, &t, false), Plan { launch: with_cwd(launch("code", &["--goto", "/r/a.rs:12:5"])), toast: None });
        assert_eq!(plan(&code, &t, true), Plan { launch: with_cwd(launch("xdg-open", &["/r/a.rs"])), toast: None });
        let page = Target { cwd: Some(cwd), ..target("/r/index.html") };
        assert_eq!(plan(&code, &page, false).launch, with_cwd(launch("xdg-open", &["/r/index.html"])));
        let png = Target { binary: true, ..target("/r/a.png") };
        assert_eq!(plan(&host(&[("VISUAL", "code")], &[], true), &png, false).launch, launch("open", &["/r/a.png"]));
        // terminal editor in the first terminal on PATH
        let nvim = host(&[("EDITOR", "nvim")], &["foot"], false);
        assert_eq!(plan(&nvim, &t, false), Plan { launch: with_cwd(launch("foot", &["nvim", "+12", "/r/a.rs"])), toast: None });
    }

    #[test]
    fn an_unknown_editor_from_editor_runs_in_a_terminal() {
        // $EDITOR names a terminal editor by convention; a wrapper like `omarchy-launch-editor --inline` runs one inline
        let t = target("/r/a.rs");
        let wrapper = host(&[("EDITOR", "omarchy-launch-editor --inline")], &["foot"], false);
        assert_eq!(plan(&wrapper, &t, false).launch, launch("foot", &["omarchy-launch-editor", "--inline", "/r/a.rs"]));
        // $VISUAL and LENZ_EDITOR name GUI editors; known GUI editors never get a terminal
        assert_eq!(plan(&host(&[("VISUAL", "my-gui-editor")], &["foot"], false), &t, false).launch, launch("my-gui-editor", &["/r/a.rs"]));
        assert_eq!(plan(&host(&[("EDITOR", "code")], &["foot"], false), &t, false).launch, launch("code", &["--goto", "/r/a.rs:12:5"]));
    }

    #[test]
    fn the_linux_default_app_goes_through_gio_when_installed() {
        // gio wraps a terminal default app (nvim.desktop) in a terminal; xdg-open would start it with no window
        let t = target("/r/settings.toml");
        assert_eq!(plan(&host(&[], &["gio"], false), &t, true).launch, launch("gio", &["open", "/r/settings.toml"]));
        assert_eq!(plan(&host(&[], &[], false), &t, true).launch, launch("xdg-open", &["/r/settings.toml"]));
        assert_eq!(plan(&host(&[], &["gio"], true), &t, true).launch, launch("open", &["/r/settings.toml"]));
    }

    #[test]
    fn plan_falls_back_to_the_system_default_with_a_toast() {
        let none = plan(&host(&[], &["foot"], false), &target("/r/a.rs"), false);
        assert_eq!(none, Plan { launch: launch("xdg-open", &["/r/a.rs"]), toast: Some(NO_EDITOR_TOAST.into()) });
        let no_terminal = plan(&host(&[("EDITOR", "nvim")], &[], false), &target("/r/a.rs"), false);
        assert_eq!(no_terminal.launch, launch("xdg-open", &["/r/a.rs"]));
        assert_eq!(no_terminal.toast.as_deref(), Some("No terminal found to run nvim; set $TERMINAL"));
    }

}
